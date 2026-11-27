// 随影 ShadeBar — 注入载荷 ShadeBarTap.dll
//
// 职责：在 explorer 进程内通过 XAML 诊断接口，按宿主下发的配色指令
//       修改任务栏「时间 / 日期 / 托盘系统字形图标 / 图标状态条」的前景色。
//
// 铁律（全部来自真机验证，违反必崩）：
//   1. 只用 IXamlDiagnostics / IVisualTreeService3 官方接口，禁止任何机器码补丁 / inline hook
//   2. AdviseVisualTreeChange 必须在 XAML UI 线程调用，否则 E_UNEXPECTED
//   3. 一切属性读写必须经 ICoreDispatcher::RunAsync 回 UI 线程，否则 RPC_E_WRONG_THREAD
//   4. 回调用对象必须实现 IAgileObject，否则 RunAsync = E_INVALIDARG
//   5. 属性索引只能从 GetPropertyValuesChain 的 PropertyChainValue.Index 取
//   6. 元素句柄只当次有效；每次应用前重新按 Name 扫描，禁止跨时点持有
//   7. 订阅失败立即停止一切树操作
//   8. 退出时 ClearProperty 还原，并 UnadviseVisualTreeChange

#define WIN32_LEAN_AND_MEAN
#define SHADEBAR_PAYLOAD_BUILD __DATE__ " " __TIME__
#include <windows.h>
#include <xamlom.h>
#include <windows.foundation.h>
#include <windows.ui.core.h>
#include <stdio.h>
#include <vector>
#include <string>
#include <map>

#include "ShadeBarIpc.h"

#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "oleaut32.lib")

// ---------------------------------------------------------------- 日志

static wchar_t g_logPath[MAX_PATH] = L"";
static CRITICAL_SECTION g_logCs;
static INIT_ONCE g_logOnce = INIT_ONCE_STATIC_INIT;
static LONG g_logBytes = 0;

static BOOL CALLBACK LogInit_(PINIT_ONCE, PVOID, PVOID*) {
    InitializeCriticalSection(&g_logCs);
    return TRUE;
}

static void LogW(const wchar_t* fmt, ...) {
    InitOnceExecuteOnce(&g_logOnce, LogInit_, nullptr, nullptr);
    if (!g_logPath[0]) return;

    wchar_t buf[2048];
    va_list ap;
    va_start(ap, fmt);
    _vsnwprintf_s(buf, _countof(buf), _TRUNCATE, fmt, ap);
    va_end(ap);

    // 时间戳：开机故障排查时，没有时间就无法把载荷日志与宿主日志对齐。
    wchar_t stamped[2160];
    SYSTEMTIME st{};
    GetLocalTime(&st);
    _snwprintf_s(stamped, _countof(stamped), _TRUNCATE,
                 L"[%02u:%02u:%02u.%03u] %s",
                 st.wHour, st.wMinute, st.wSecond, st.wMilliseconds, buf);

    char utf8[7168];
    int n = WideCharToMultiByte(CP_UTF8, 0, stamped, -1, utf8, sizeof(utf8) - 2, nullptr, nullptr);
    if (n <= 0) return;
    utf8[n - 1] = '\n';

    EnterCriticalSection(&g_logCs);
    if (g_logBytes > 262144) {
        FILE* t = _wfopen(g_logPath, L"wb");
        if (t) fclose(t);
        g_logBytes = 0;
    }
    FILE* f = _wfopen(g_logPath, L"ab");
    if (f) {
        fwrite(utf8, 1, (size_t)n, f);
        fclose(f);
        g_logBytes += n;
    }
    LeaveCriticalSection(&g_logCs);
}

// ---------------------------------------------------------------- 全局状态

static HMODULE g_self = nullptr;
static IXamlDiagnostics*   g_diag = nullptr;
static IVisualTreeService3* g_vts3 = nullptr;
static IVisualTreeService2* g_vts2 = nullptr;

static HANDLE g_advisedEvent = nullptr;
static HANDLE g_uiDoneEvent = nullptr;
static volatile LONG g_advised = 0;
static volatile LONG g_dirty = 1;
static volatile LONG g_applied = 0;

struct VisEntry {
    InstanceHandle handle;
    InstanceHandle parent;
    std::wstring   type;
    std::wstring   name;
};
static std::vector<VisEntry> g_tree;
// 句柄 → 类型。用来判断某个 TextBlock 的父级是不是 SystemTray.TextIconContent
// （托盘系统字形），仅靠名字无法区分它和应用图标的文字层。
static std::map<InstanceHandle, std::wstring> g_typeMap;
// 句柄 → 名字。托盘字形只取 AdaptiveTextBlock 里名为 Base 的那一层
// （Underlay/AccentOverlay 是透明/强调覆盖层，宽 0，不该染色）。
static std::map<InstanceHandle, std::wstring> g_nameMap;
// "显示隐藏的图标"箭头控件（SystemTray.ChevronIconView）。它没有 XAML 子元素，
// 只能直接给控件本身写 Foreground，保留最近一次句柄用于属性诊断。
static InstanceHandle g_chevronHandle = 0;
static volatile LONG  g_chevronDumped = 0;
// v1.5.5 取证：chevron 的完整生命周期事件（不受诊断预算限制，事件本身极少）。
// 冷开机箭头不着色的根因未明：既可能是 Add 事件没到（没进树），也可能是
// 写进了 Foreground 但 WinComp 内部绘制不读它。下面两个计数 + 回读日志用于区分。
static volatile LONG  g_chevronAdds = 0;
static volatile LONG  g_chevronRemoves = 0;
static CRITICAL_SECTION      g_treeCs;
static INIT_ONCE             g_treeOnce = INIT_ONCE_STATIC_INIT;

static BOOL CALLBACK TreeInit_(PINIT_ONCE, PVOID, PVOID*) {
    InitializeCriticalSection(&g_treeCs);
    return TRUE;
}

static ShadeBarIpc* g_ipc = nullptr;

// 微秒级计时：GetTickCount64 分辨率约 15.6 ms，重放明明很快也会显示 0/16。
static ULONGLONG NowUs() {
    static LARGE_INTEGER freq{};
    if (freq.QuadPart == 0) QueryPerformanceFrequency(&freq);
    LARGE_INTEGER c{};
    QueryPerformanceCounter(&c);
    return (ULONGLONG)(c.QuadPart * 1000000ULL / freq.QuadPart);
}

// 最近 15 s 内是否有键盘/鼠标输入。用来给"树重放"定节奏：
// 有新窗口 = 用户正在操作；空闲时没必要每 5 s 扫一遍整棵树。
static bool UserActiveRecently() {
    LASTINPUTINFO li{};
    li.cbSize = sizeof(li);
    if (!GetLastInputInfo(&li)) return true;
    return (GetTickCount() - li.dwTime) < 15000;
}
/* 代际令牌：同一 explorer 里可能同时驻留多个版本的载荷（升级、重复注入）。
 * 它们共享同一块 IPC，谁都能写 —— 旧副本会用旧逻辑持续覆盖新副本的结果，
 * 现象就是"改了没反应"。约定：最后注入的实例把 reserved[1] 递增并记为自己的
 * 代际；其余实例发现代际不匹配就彻底停手，且**不做还原**（还原会抹掉新副本）。 */
static unsigned g_generation = 0;
// 是否真的向任务栏写过颜色。未写过时禁止执行还原：
// RunningIndicator 的 Fill 一旦被 ClearProperty 会变全透明（v1.1 生产事故）。
static volatile LONG g_touched = 0;

/* 元素来源（订阅一次，不再重放）
 *
 * XAML 诊断接口提供两条途径：订阅时的**一次全量投递**，以及之后的 Add/Remove
 * 增量事件；没有"遍历整棵树"的接口。
 *
 * v1.3.1 曾用 Unadvise + Advise 重放来补新窗口的元素，但 v1.5.2 实测：
 * 第一次 Advise 成功后，之后任何一次 Advise（无论先退订、还是换新回调对象）
 * 都会返回 E_UNEXPECTED，频繁重放还会把整个诊断连接弄坏，连时间/日期都不再着色。
 * 而保持初始订阅时，新打开的应用窗口**确实会送来 Add 事件**
 * （打开记事本后 ind 5→6，期间没有任何重放）。
 *
 * 因此现在只建立一次订阅，完全依赖 Add/Remove 增量；不重放、不重建会话。
 * 冷启动最坏只是"稍晚收到元素"，不会再让整条链路失效。
 */
static volatile LONG g_clearOnAdd = 0;   // 首次订阅的第一个 Add 到来时清空旧树
// 指示条选中态缓存（UI 线程独占）。实测 OnElementStateChanged 对指示条的
// "选中 ↔ 正常"切换**根本不触发**，g_dirty 没机会置位，只能等宿主下一条指令
// （约 2 s）才改色。这里由 UI 线程定时只读 ActualWidth，发现变化就置脏。
static std::map<InstanceHandle, bool> g_indSelState;
static volatile LONG g_urgent = 0;       // 1 = 绕过 250 ms 限流，立即应用
static int g_addLogBudget = 200;         // Add 事件诊断日志预算（防刷屏）
// 冷开机取证：Add 事件全局计数（不受日志预算限制）。200 条日志不代表只有 200 个
// Add——必须区分"新窗口的 Add 根本没来"和"来了但被日志预算截断没打出来"。
static volatile LONG g_addTotal = 0;
static volatile LONG g_addIndTotal = 0;  // 其中 name=RunningIndicator 的个数
// 关键事件每 N 个打一行摘要，既能看总量又能看新元素是否持续到达。
static volatile LONG g_addTicker = 0;
static int g_trayExtraLogBudget = 60;    // 托盘扩展元素（箭头）命中日志预算
static int g_indFailBudget = 30;         // 指示条写入失败日志预算

// 定义在下方（共享画刷一节）；重放/还原后必须丢弃句柄，这里先声明。
static void ResetSharedBrush();
static bool SetBrushColorDirect(InstanceHandle brushH, unsigned argb);

static std::wstring GuidToStr(REFIID r) {
    wchar_t b[64];
    StringFromGUID2(r, b, 64);
    return b;
}

static IVisualTreeService* VtsBase() {
    if (g_vts3) return static_cast<IVisualTreeService*>(static_cast<IVisualTreeService2*>(g_vts3));
    if (g_vts2) return static_cast<IVisualTreeService*>(g_vts2);
    return nullptr;
}

// ---------------------------------------------------------------- TAP 对象

class TapObject;

// 当前订阅使用的回调对象。重放时会换成一个新的对象（先订阅成功再拆旧的），
// 因此不能只由 IClassFactory 持有。
static TapObject* g_tap = nullptr;

class TapObject final : public IObjectWithSite, public IVisualTreeServiceCallback2 {
public:
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        if (riid == __uuidof(IUnknown) || riid == __uuidof(IObjectWithSite)) {
            *ppv = static_cast<IObjectWithSite*>(this); AddRef(); return S_OK;
        }
        if (riid == __uuidof(IVisualTreeServiceCallback2) || riid == __uuidof(IVisualTreeServiceCallback)) {
            *ppv = static_cast<IVisualTreeServiceCallback2*>(this); AddRef(); return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return InterlockedIncrement(&m_ref); }
    ULONG STDMETHODCALLTYPE Release() override {
        ULONG r = InterlockedDecrement(&m_ref);
        if (r == 0) delete this;
        return r;
    }

    HRESULT STDMETHODCALLTYPE SetSite(IUnknown* site) override {
        if (!site) return S_OK;
        LogW(L"[Tap] SetSite tid=%lu", GetCurrentThreadId());

        IXamlDiagnostics* d = nullptr;
        if (SUCCEEDED(site->QueryInterface(__uuidof(IXamlDiagnostics), (void**)&d))) g_diag = d;

        IVisualTreeService3* v3 = nullptr;
        if (SUCCEEDED(site->QueryInterface(__uuidof(IVisualTreeService3), (void**)&v3))) g_vts3 = v3;

        IVisualTreeService2* v2 = nullptr;
        if (SUCCEEDED(site->QueryInterface(__uuidof(IVisualTreeService2), (void**)&v2))) g_vts2 = v2;

        if (!g_vts3 && !g_vts2) {
            LogW(L"[Tap] FATAL: site 无 IVisualTreeService");
        } else if (!g_advised) {
            HRESULT hr = g_vts3 ? g_vts3->AdviseVisualTreeChange(this)
                                : g_vts2->AdviseVisualTreeChange(this);
            LogW(L"[Tap] Advise hr=0x%08X", (unsigned)hr);
            if (SUCCEEDED(hr)) {
                InterlockedExchange(&g_advised, 1);
                if (g_advisedEvent) SetEvent(g_advisedEvent);
            } else {
                LogW(L"[Tap] 订阅失败，本进程放弃改色");
            }
        }
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetSite(REFIID, void** ppv) override {
        if (ppv) *ppv = nullptr;
        return E_NOTIMPL;
    }

    HRESULT STDMETHODCALLTYPE OnVisualTreeChange(
        ParentChildRelation relation, VisualElement element, VisualMutationType type) override {
        InitOnceExecuteOnce(&g_treeOnce, TreeInit_, nullptr, nullptr);
        EnterCriticalSection(&g_treeCs);
        if (type == Add) {
            // 重放开始：第一个 Add 到达时清掉旧树，之后重新累积。
            if (InterlockedExchange(&g_clearOnAdd, 0)) {
                g_tree.clear(); g_typeMap.clear(); g_nameMap.clear();
            }

            VisEntry e;
            e.handle = element.Handle;
            e.parent = relation.Parent;
            e.type = element.Type ? element.Type : L"";
            e.name = element.Name ? element.Name : L"";
            const std::wstring& N = e.name;
            const std::wstring& T = e.type;
            g_typeMap[e.handle] = T;
            g_nameMap[e.handle] = N;


            // 取证：先无条件计数，再决定是否打日志。RunningIndicator 是本次要查的
            // 目标元素，每次都打（不受 200 条预算限制）；其余走原预算 + 定期摘要。
            const long addN = InterlockedIncrement(&g_addTotal);
            const bool isInd = (N == L"RunningIndicator");
            if (isInd) InterlockedIncrement(&g_addIndTotal);
            if (isInd || g_addLogBudget > 0) {
                if (!isInd) --g_addLogBudget;
                LogW(L"[Add] name=\"%s\" type=\"%s\" parent=0x%llX%s",
                     N.c_str(), T.c_str(), (unsigned long long)relation.Parent,
                     isInd ? L" <<IND>>" : L"");
            }
            const long tick = InterlockedIncrement(&g_addTicker);
            if ((tick % 100) == 0) {
                size_t inTreeInd = 0;
                for (const auto& it : g_tree) if (it.name == L"RunningIndicator") ++inTreeInd;
                LogW(L"[Diag] ADD total=%ld ind=%ld(inTree=%zu)",
                     addN, (long)g_addIndTotal, inTreeInd);
            }

            bool want = (N == L"TimeInnerTextBlock" || N == L"DateInnerTextBlock" ||
                         N == L"RunningIndicator");
            if (!want && N == L"InnerTextBlock") {
                // 托盘字形：只收 AdaptiveTextBlock 的 Base 层子 TextBlock。
                auto pn = g_nameMap.find(relation.Parent);
                want = (pn != g_nameMap.end() && pn->second == L"Base");
            }
            // "显示隐藏的图标"箭头：实测 SystemTray.ChevronIconView **没有任何 XAML 子元素**
            // （字形由 WinComp 内部绘制），因此不能靠子 TextBlock 染色，只能直接给这个
            // 控件本身写 Foreground。归入 InnerTextBlock 通道，复用托盘的着色 / 还原逻辑。
            if (!want && T == L"SystemTray.ChevronIconView") {
                want = true;
                e.name = L"InnerTextBlock";
                g_chevronHandle = element.Handle;
                const long adds = InterlockedIncrement(&g_chevronAdds);
                LogW(L"[Diag] chevron ADD #%ld h=0x%llX parent=0x%llX",
                     adds, (unsigned long long)element.Handle,
                     (unsigned long long)relation.Parent);
                if (g_trayExtraLogBudget > 0) {
                    --g_trayExtraLogBudget;
                    LogW(L"[Tray+] chevron handle=0x%llX parent=0x%llX",
                         (unsigned long long)element.Handle,
                         (unsigned long long)relation.Parent);
                }
            }
            if (want) {
                // 重放时同一个句柄可能被再发一次，去重，避免 g_tree 无限增长。
                bool dup = false;
                for (auto& it : g_tree) {
                    if (it.handle == e.handle) { it = e; dup = true; break; }
                }
                if (!dup) g_tree.push_back(e);
                InterlockedExchange(&g_dirty, 1);
            }
        } else {
            g_typeMap.erase(element.Handle);
            g_nameMap.erase(element.Handle);
            if (element.Handle == g_chevronHandle) {
                const long rems = InterlockedIncrement(&g_chevronRemoves);
                LogW(L"[Diag] chevron REMOVE #%ld h=0x%llX",
                     rems, (unsigned long long)element.Handle);
                g_chevronHandle = 0;
            }
            for (auto it = g_tree.begin(); it != g_tree.end(); ++it) {
                if (it->handle == element.Handle) { g_tree.erase(it); break; }
            }
        }
        LeaveCriticalSection(&g_treeCs);
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE OnElementStateChanged(InstanceHandle, VisualElementState, LPCWSTR) override {
        // 正常态 ↔ 选中态 的切换**不产生 Add 事件**，只会走到这里。
        // 不置脏的话载荷永远不会重扫，表现就是"切到选中后指示条退回系统色"。
        InterlockedExchange(&g_dirty, 1);
        return S_OK;
    }

private:
    LONG m_ref = 1;
};

class TapFactory final : public IClassFactory {
public:
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        if (riid == __uuidof(IUnknown) || riid == __uuidof(IClassFactory)) {
            *ppv = static_cast<IClassFactory*>(this); AddRef(); return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return 2; }
    ULONG STDMETHODCALLTYPE Release() override { return 1; }
    HRESULT STDMETHODCALLTYPE CreateInstance(IUnknown* outer, REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        if (outer) return CLASS_E_NOAGGREGATION;
        if (!g_tap) g_tap = new (std::nothrow) TapObject();
        if (!g_tap) return E_OUTOFMEMORY;
        return g_tap->QueryInterface(riid, ppv);
    }
    HRESULT STDMETHODCALLTYPE LockServer(BOOL) override { return S_OK; }
};

// ---------------------------------------------------------------- 元素操作

static std::vector<VisEntry> Snapshot() {
    EnterCriticalSection(&g_treeCs);
    std::vector<VisEntry> c = g_tree;
    LeaveCriticalSection(&g_treeCs);
    return c;
}

static bool IndexFromChain(InstanceHandle h, const wchar_t* propName, unsigned int* outIdx) {
    IVisualTreeService* svc = VtsBase();
    if (!svc) return false;
    unsigned int srcCount = 0, propCount = 0;
    PropertyChainSource* sources = nullptr;
    PropertyChainValue* props = nullptr;
    HRESULT hr = svc->GetPropertyValuesChain(h, &srcCount, &sources, &propCount, &props);
    bool ok = false;
    if (SUCCEEDED(hr)) {
        for (unsigned int i = 0; i < propCount; ++i) {
            if (props[i].PropertyName && wcscmp(props[i].PropertyName, propName) == 0) {
                *outIdx = props[i].Index;
                ok = true;
                break;
            }
        }
        for (unsigned int i = 0; i < propCount; ++i) {
            SysFreeString(props[i].Type); SysFreeString(props[i].DeclaringType);
            SysFreeString(props[i].ValueType); SysFreeString(props[i].ItemType);
            SysFreeString(props[i].Value); SysFreeString(props[i].PropertyName);
        }
        for (unsigned int i = 0; i < srcCount; ++i) {
            SysFreeString(sources[i].TargetType); SysFreeString(sources[i].Name);
        }
        if (props) CoTaskMemFree(props);
        if (sources) CoTaskMemFree(sources);
    }
    return ok;
}

// 读一个 double 数值属性（用于判断状态条正常/选中）
static bool ReadNumber(InstanceHandle h, const wchar_t* propName, double* out) {
    IVisualTreeService* svc = VtsBase();
    if (!svc) return false;
    unsigned int srcCount = 0, propCount = 0;
    PropertyChainSource* sources = nullptr;
    PropertyChainValue* props = nullptr;
    HRESULT hr = svc->GetPropertyValuesChain(h, &srcCount, &sources, &propCount, &props);
    bool ok = false;
    if (SUCCEEDED(hr)) {
        for (unsigned int i = 0; i < propCount; ++i) {
            if (props[i].PropertyName && wcscmp(props[i].PropertyName, propName) == 0 && props[i].Value) {
                *out = _wtof(props[i].Value);
                ok = true;
                break;
            }
        }
        for (unsigned int i = 0; i < propCount; ++i) {
            SysFreeString(props[i].Type); SysFreeString(props[i].DeclaringType);
            SysFreeString(props[i].ValueType); SysFreeString(props[i].ItemType);
            SysFreeString(props[i].Value); SysFreeString(props[i].PropertyName);
        }
        for (unsigned int i = 0; i < srcCount; ++i) {
            SysFreeString(sources[i].TargetType); SysFreeString(sources[i].Name);
        }
        if (props) CoTaskMemFree(props);
        if (sources) CoTaskMemFree(sources);
    }
    return ok;
}

// 解析 "#AARRGGBB"
static bool ParseColorStr(const wchar_t* v, unsigned* out) {
    if (!v || v[0] != L'#') return false;
    wchar_t* end = nullptr;
    unsigned long val = wcstoul(v + 1, &end, 16);
    if (!end || *end != L'\0') return false;
    *out = (unsigned)val;
    return true;
}

// 读取元素某画刷属性的「当前有效颜色」（用于还原，绝不能只靠 ClearProperty：
// RunningIndicator 的 Fill 由视觉状态驱动，清空后会变成透明）
static bool ReadBrushColor(InstanceHandle elem, const wchar_t* propName, unsigned* outArgb) {
    IVisualTreeService* svc = VtsBase();
    if (!svc || !elem || !outArgb) return false;

    InstanceHandle brushH = 0;
    {
        unsigned int sc = 0, pc = 0;
        PropertyChainSource* src = nullptr;
        PropertyChainValue* pr = nullptr;
        HRESULT hr = svc->GetPropertyValuesChain(elem, &sc, &src, &pc, &pr);
        if (FAILED(hr)) return false;
        for (unsigned int i = 0; i < pc; ++i) {
            if (pr[i].PropertyName && wcscmp(pr[i].PropertyName, propName) == 0 &&
                pr[i].Value && brushH == 0) {
                InstanceHandle cand = (InstanceHandle)_wtoi64(pr[i].Value);
                if (cand) brushH = cand;
            }
        }
        for (unsigned int i = 0; i < pc; ++i) {
            SysFreeString(pr[i].Type); SysFreeString(pr[i].DeclaringType);
            SysFreeString(pr[i].ValueType); SysFreeString(pr[i].ItemType);
            SysFreeString(pr[i].Value); SysFreeString(pr[i].PropertyName);
        }
        for (unsigned int i = 0; i < sc; ++i) { SysFreeString(src[i].TargetType); SysFreeString(src[i].Name); }
        if (pr) CoTaskMemFree(pr);
        if (src) CoTaskMemFree(src);
    }
    if (!brushH) return false;

    bool ok = false;
    unsigned int sc2 = 0, pc2 = 0;
    PropertyChainSource* src2 = nullptr;
    PropertyChainValue* pr2 = nullptr;
    HRESULT hr2 = svc->GetPropertyValuesChain(brushH, &sc2, &src2, &pc2, &pr2);
    if (SUCCEEDED(hr2)) {
        for (unsigned int i = 0; i < pc2; ++i) {
            if (pr2[i].PropertyName && wcscmp(pr2[i].PropertyName, L"Color") == 0 && pr2[i].Value) {
                unsigned argb = 0;
                if (ParseColorStr(pr2[i].Value, &argb)) {
                    // 跳过全透明项，取第一个有 alpha 的颜色
                    if (((argb >> 24) & 0xFF) != 0 || !ok) { *outArgb = argb; ok = true; }
                    if (ok && ((argb >> 24) & 0xFF) != 0) break;
                }
            }
        }
        for (unsigned int i = 0; i < pc2; ++i) {
            SysFreeString(pr2[i].Type); SysFreeString(pr2[i].DeclaringType);
            SysFreeString(pr2[i].ValueType); SysFreeString(pr2[i].ItemType);
            SysFreeString(pr2[i].Value); SysFreeString(pr2[i].PropertyName);
        }
        for (unsigned int i = 0; i < sc2; ++i) { SysFreeString(src2[i].TargetType); SysFreeString(src2[i].Name); }
        if (pr2) CoTaskMemFree(pr2);
        if (src2) CoTaskMemFree(src2);
    }
    return ok;
}

// 原始颜色缓存（首次应用前抓取，退出时写回）
struct RestoreColors {
    unsigned time = 0;
    unsigned date = 0;
    unsigned tray = 0;
    unsigned indNormal = 0;
    unsigned indSelected = 0;
    // 选中态那条 Fill 指向的画刷对象（退出时即使已无图标处于选中态，也要把它改回来）
    InstanceHandle indSelectedBrush = 0;
};
static RestoreColors g_restore;
// 我们最后一次写进指示条的色值，供退出时按内容识别"哪些画刷是我们染的"。
static unsigned g_appliedIndColor = 0;
// 选中态那份（正常/选中现在可以是不同颜色，兜底识别要同时认两种）。
static unsigned g_appliedIndSelectedColor = 0;

static bool SetColorProp(InstanceHandle h, const wchar_t* propName, unsigned argb) {
    IVisualTreeService* svc = VtsBase();
    if (!svc || !h) return false;
    unsigned int idx = 0;
    if (!IndexFromChain(h, propName, &idx)) return false;

    wchar_t colorStr[16];
    swprintf_s(colorStr, L"#%08X", argb);

    InstanceHandle brush = 0;
    BSTR tn = SysAllocString(L"Windows.UI.Xaml.Media.SolidColorBrush");
    BSTR val = SysAllocString(colorStr);
    HRESULT hr = svc->CreateInstance(tn, val, &brush);
    SysFreeString(tn); SysFreeString(val);
    if (FAILED(hr) || !brush) return false;

    hr = svc->SetProperty(h, brush, idx);
    return SUCCEEDED(hr);
}

// ---------------------------------------------------------------- 共享指示条画刷
//
// 用户选定方案 B：所有"正常态"指示条指向**同一个** SolidColorBrush，
// 之后只改这一个对象的 Color，全部指示条就在同一刻变色，不会出现"改了一半"。
//
// 三条约束（都来自现有机制）：
//   * 画刷句柄在树重放 / 还原后可能失效 → 一律丢弃重建（ResetSharedBrush）；
//   * 选中态会被 VisualState 换成它自己的画刷，仍走原有的逐条写入；
//   * 还原时逐条写回原色（会替换掉共享画刷），所以还原后也要丢弃句柄。
static InstanceHandle g_sharedIndBrush = 0;
static unsigned       g_sharedIndColor = 0;

static void ResetSharedBrush() {
    g_sharedIndBrush = 0;
    g_sharedIndColor = 0;
}

static InstanceHandle NewSolidBrush(unsigned argb) {
    IVisualTreeService* svc = VtsBase();
    if (!svc) return 0;
    wchar_t colorStr[16];
    swprintf_s(colorStr, L"#%08X", argb);
    InstanceHandle brush = 0;
    BSTR tn = SysAllocString(L"Windows.UI.Xaml.Media.SolidColorBrush");
    BSTR val = SysAllocString(colorStr);
    HRESULT hr = svc->CreateInstance(tn, val, &brush);
    SysFreeString(tn); SysFreeString(val);
    return SUCCEEDED(hr) ? brush : 0;
}

/// 把已有画刷对象赋给元素的画刷属性（不新建画刷）。
static bool SetBrushProp(InstanceHandle elem, const wchar_t* propName, InstanceHandle brush) {
    IVisualTreeService* svc = VtsBase();
    if (!svc || !elem || !brush) return false;
    unsigned int idx = 0;
    if (!IndexFromChain(elem, propName, &idx)) return false;
    return SUCCEEDED(svc->SetProperty(elem, brush, idx));
}

/// 取共享画刷并保证它的颜色是 argb；返回句柄（0 = 失败）。
static InstanceHandle EnsureSharedBrush(unsigned argb) {
    if (g_sharedIndBrush && g_sharedIndColor == argb) return g_sharedIndBrush;
    if (g_sharedIndBrush && SetBrushColorDirect(g_sharedIndBrush, argb)) {
        // 只改这一个对象：所有指向它的指示条同刻变色
        g_sharedIndColor = argb;
        return g_sharedIndBrush;
    }
    InstanceHandle b = NewSolidBrush(argb);
    if (!b) { ResetSharedBrush(); return 0; }
    g_sharedIndBrush = b;
    g_sharedIndColor = argb;
    return b;
}

static bool ClearColorProp(InstanceHandle h, const wchar_t* propName) {
    IVisualTreeService* svc = VtsBase();
    if (!svc || !h) return false;
    unsigned int idx = 0;
    if (!IndexFromChain(h, propName, &idx)) return false;
    return SUCCEEDED(svc->ClearProperty(h, idx));
}

// 取某属性链里第一个非空的画刷句柄。
// 选中态的 Fill 有两个 Local 槽：VisualState 写的那个排在前面（优先级更高），
// 因此这里取到的就是系统强调色画刷，而不是我们自己写进去的那个。
static InstanceHandle BrushHandleOfProp(InstanceHandle elem, const wchar_t* propName) {
    IVisualTreeService* svc = VtsBase();
    if (!svc || !elem) return 0;
    unsigned int sc = 0, pc = 0;
    PropertyChainSource* src = nullptr;
    PropertyChainValue* pr = nullptr;
    InstanceHandle out = 0;
    if (SUCCEEDED(svc->GetPropertyValuesChain(elem, &sc, &src, &pc, &pr))) {
        for (unsigned int i = 0; i < pc; ++i) {
            if (pr[i].PropertyName && wcscmp(pr[i].PropertyName, propName) == 0 && pr[i].Value) {
                InstanceHandle cand = (InstanceHandle)_wtoi64(pr[i].Value);
                if (cand) { out = cand; break; }
            }
        }
        for (unsigned int i = 0; i < pc; ++i) {
            SysFreeString(pr[i].Type); SysFreeString(pr[i].DeclaringType);
            SysFreeString(pr[i].ValueType); SysFreeString(pr[i].ItemType);
            SysFreeString(pr[i].Value); SysFreeString(pr[i].PropertyName);
        }
        for (unsigned int i = 0; i < sc; ++i) { SysFreeString(src[i].TargetType); SysFreeString(src[i].Name); }
        if (pr) CoTaskMemFree(pr);
        if (src) CoTaskMemFree(src);
    }
    return out;
}

// 读画刷对象自身的 Color（不是从元素属性链绕）。
static bool ReadColorOfBrush(InstanceHandle brushH, unsigned* outArgb) {
    IVisualTreeService* svc = VtsBase();
    if (!svc || !brushH || !outArgb) return false;
    unsigned int sc = 0, pc = 0;
    PropertyChainSource* src = nullptr;
    PropertyChainValue* pr = nullptr;
    bool ok = false;
    if (SUCCEEDED(svc->GetPropertyValuesChain(brushH, &sc, &src, &pc, &pr))) {
        for (unsigned int i = 0; i < pc; ++i) {
            if (pr[i].PropertyName && wcscmp(pr[i].PropertyName, L"Color") == 0 && pr[i].Value) {
                unsigned a = 0;
                if (ParseColorStr(pr[i].Value, &a)) { *outArgb = a; ok = true; break; }
            }
        }
        for (unsigned int i = 0; i < pc; ++i) {
            SysFreeString(pr[i].Type); SysFreeString(pr[i].DeclaringType);
            SysFreeString(pr[i].ValueType); SysFreeString(pr[i].ItemType);
            SysFreeString(pr[i].Value); SysFreeString(pr[i].PropertyName);
        }
        for (unsigned int i = 0; i < sc; ++i) { SysFreeString(src[i].TargetType); SysFreeString(src[i].Name); }
        if (pr) CoTaskMemFree(pr);
        if (src) CoTaskMemFree(src);
    }
    return ok;
}

// 直接改画刷对象的 Color。
// 为什么必须这么做：选中态下 VisualState 会往 Local 层插一个更高优先级的 Fill 槽，
// SetProperty 写元素的 Fill 永远排在它后面 —— 真机实测 6 条正常态全部生效、
// 唯独 ActualWidth=16 的选中态那条纹丝不动（读回有值、视觉不变）。
// 而画刷对象是共用的，改它自身的 Color 不经过属性优先级，可以穿透。
static bool SetBrushColorDirect(InstanceHandle brushH, unsigned argb) {
    IVisualTreeService* svc = VtsBase();
    if (!svc || !brushH) return false;
    unsigned int idx = 0;
    if (!IndexFromChain(brushH, L"Color", &idx)) return false;
    wchar_t s[16];
    swprintf_s(s, L"#%08X", argb);
    InstanceHandle colorObj = 0;
    BSTR tn = SysAllocString(L"Windows.UI.Color");
    BSTR v = SysAllocString(s);
    HRESULT hr = svc->CreateInstance(tn, v, &colorObj);
    SysFreeString(tn); SysFreeString(v);
    if (FAILED(hr) || !colorObj) return false;
    return SUCCEEDED(svc->SetProperty(brushH, colorObj, idx));
}

static bool SetBrushColorOfProp(InstanceHandle elem, const wchar_t* propName, unsigned argb) {
    InstanceHandle bh = BrushHandleOfProp(elem, propName);
    return bh ? SetBrushColorDirect(bh, argb) : false;
}

// 把某一属性**所有层**引用的画刷都染成同一色。
// 只在"选中态"用：选中态 Fill 有 3 层，属性链首层未必是渲染实际取用的那一层；
// 选中态画刷引用计数为 1，逐层染色无副作用。正常态画刷被数十处共享，禁止用本函数。
static int SetAllBrushColorsOfProp(InstanceHandle elem, const wchar_t* propName, unsigned argb) {
    IVisualTreeService* svc = VtsBase();
    if (!svc) return 0;
    unsigned int sc = 0, pc = 0;
    PropertyChainSource* src = nullptr;
    PropertyChainValue* pr = nullptr;
    int n = 0;
    if (SUCCEEDED(svc->GetPropertyValuesChain(elem, &sc, &src, &pc, &pr))) {
        for (unsigned int i = 0; i < pc; ++i) {
            if (pr[i].PropertyName && wcscmp(pr[i].PropertyName, propName) == 0 && pr[i].Value) {
                InstanceHandle cand = (InstanceHandle)_wtoi64(pr[i].Value);
                if (cand && SetBrushColorDirect(cand, argb)) ++n;
            }
        }
        for (unsigned int i = 0; i < pc; ++i) {
            SysFreeString(pr[i].Type); SysFreeString(pr[i].DeclaringType);
            SysFreeString(pr[i].ValueType); SysFreeString(pr[i].ItemType);
            SysFreeString(pr[i].Value); SysFreeString(pr[i].PropertyName);
        }
        for (unsigned int i = 0; i < sc; ++i) { SysFreeString(src[i].TargetType); SysFreeString(src[i].Name); }
        if (pr) CoTaskMemFree(pr);
        if (src) CoTaskMemFree(src);
    }
    return n;
}

// ---------------------------------------------------------------- UI 线程工作

/* 元素掩码位，必须与宿主 recolor::elem_mask 一致 */
static const unsigned kMaskTime = 1u << 0;
static const unsigned kMaskDate = 1u << 1;
static const unsigned kMaskTray = 1u << 2;
static const unsigned kMaskIndicator = 1u << 3;

/* 上次生效的掩码：只在掩码位关闭时把元素写回原色，避免每拍重复写。 */
static unsigned g_activeMask = 0;

static void ApplyColors() {
    if (!g_ipc) return;
    const ShadeBarIpc cmd = *g_ipc;
    const unsigned mask = cmd.reserved[0];

    auto v = Snapshot();
    if (g_chevronHandle && !InterlockedExchange(&g_chevronDumped, 1)) {
        unsigned idx = 0;
        const bool hasFg = IndexFromChain(g_chevronHandle, L"Foreground", &idx);
        LogW(L"[Tray+] chevron Foreground %s idx=%u",
             hasFg ? L"ok" : L"MISSING", hasFg ? idx : 0u);
    }
    // v1.5.5 取证：每拍回读 chevron 的 Foreground 实际颜色，与目标色对比。
    // 旧日志只证明"属性槽存在"（idx=876），无法区分"真染上"与"写了 WinComp 不读"。
    // 节流：前 10 拍每拍打，其后每 30 拍一次（约 60 s），避免刷屏。
    if (g_chevronHandle && cmd.trayGlyphColor != 0) {
        static volatile LONG diagSeq = 0;
        const long s = InterlockedIncrement(&diagSeq);
        if (s <= 10 || (s % 30) == 0) {
            unsigned now = 0;
            const bool rb = ReadBrushColor(g_chevronHandle, L"Foreground", &now);
            LogW(L"[Diag] chevron RB want=%08X got=%08X rb=%d adds=%ld rems=%ld",
                 cmd.trayGlyphColor, now, rb ? 1 : 0,
                 (long)g_chevronAdds, (long)g_chevronRemoves);
        }
    }
    int nTime = 0, nDate = 0, nTray = 0, nInd = 0, nUndo = 0, nShared = 0;

    // 树组成诊断：只在变化时打一行，用来区分"元素没进树"和"进了树但写不进去"。
    {
        int cT = 0, cD = 0, cG = 0, cI = 0, cO = 0;
        for (const auto& e : v) {
            if (e.name == L"TimeInnerTextBlock") ++cT;
            else if (e.name == L"DateInnerTextBlock") ++cD;
            else if (e.name == L"InnerTextBlock") ++cG;
            else if (e.name == L"RunningIndicator") ++cI;
            else ++cO;
        }
        static int lastSig = -1;
        const int sig = ((((cT * 31 + cD) * 31 + cG) * 31 + cI) * 31 + cO);
        if (sig != lastSig) {
            lastSig = sig;
            LogW(L"[Tree] n=%d time=%d date=%d tray=%d ind=%d other=%d",
                 (int)v.size(), cT, cD, cG, cI, cO);
        }
    }

    for (auto& e : v) {
        if (e.name == L"TimeInnerTextBlock") {
            if (mask & kMaskTime) {
                if (g_restore.time == 0) { unsigned a = 0; if (ReadBrushColor(e.handle, L"Foreground", &a)) g_restore.time = a; }
                if (SetColorProp(e.handle, L"Foreground", cmd.timeColor)) ++nTime;
            } else if ((g_activeMask & kMaskTime) && g_restore.time != 0) {
                // 用户刚关掉"时间"开关：写回系统原色（绝不是 ClearProperty）
                if (SetColorProp(e.handle, L"Foreground", g_restore.time)) ++nUndo;
            }
        } else if (e.name == L"DateInnerTextBlock") {
            if (mask & kMaskDate) {
                if (g_restore.date == 0) { unsigned a = 0; if (ReadBrushColor(e.handle, L"Foreground", &a)) g_restore.date = a; }
                if (SetColorProp(e.handle, L"Foreground", cmd.dateColor)) ++nDate;
            } else if ((g_activeMask & kMaskDate) && g_restore.date != 0) {
                if (SetColorProp(e.handle, L"Foreground", g_restore.date)) ++nUndo;
            }
        } else if (e.name == L"InnerTextBlock") {
            if (mask & kMaskTray) {
                if (g_restore.tray == 0) { unsigned a = 0; if (ReadBrushColor(e.handle, L"Foreground", &a)) g_restore.tray = a; }
                const bool wroteTray = SetColorProp(e.handle, L"Foreground", cmd.trayGlyphColor);
                if (wroteTray) ++nTray;
            } else if ((g_activeMask & kMaskTray) && g_restore.tray != 0) {
                if (SetColorProp(e.handle, L"Foreground", g_restore.tray)) ++nUndo;
            }
        } else if (e.name == L"RunningIndicator") {
            // Width 实测多为 "Auto"，真实宽度在 ActualWidth：
            // 正常态约 6 px、选中态约 16 px，这是区分两态的唯一可靠依据。
            double w = 0.0;
            bool selected = ReadNumber(e.handle, L"ActualWidth", &w) && w >= 10.0;
            // 顺手刷新状态缓存：本拍已经读过宽度，轮询就不必把同一状态再判成"变化"。
            g_indSelState[e.handle] = selected;

            if (mask & kMaskIndicator) {
                g_appliedIndColor = cmd.indicatorNormalColor;
                g_appliedIndSelectedColor = cmd.indicatorSelectedColor;

                // 先缓存原色（必须在任何写入之前，否则读到的是刚写的值）
                if (selected) {
                    if (g_restore.indSelected == 0) {
                        InstanceHandle bh = BrushHandleOfProp(e.handle, L"Fill");
                        unsigned a = 0;
                        if (bh && ReadColorOfBrush(bh, &a)) {
                            g_restore.indSelected = a;
                            g_restore.indSelectedBrush = bh;
                        }
                    }
                } else if (g_restore.indNormal == 0) {
                    unsigned a = 0;
                    if (ReadBrushColor(e.handle, L"Fill", &a)) g_restore.indNormal = a;
                }

                // 正常态：把**同一个**共享画刷赋给它（方案 B）。之后换色只改那个
                //        对象的 Color，全部指示条同刻变；SetProperty 本身是幂等的，
                //        重复赋同一个画刷不会造成视觉变化。
                // 选中态：VisualState 会把 Fill 换成它自己的画刷，共享画刷在这里无效，
                //        仍走"元素 Fill + 画刷 Color"双写 + 逐层染色。
                // 正常态与选中态用不同的目标色（S-52/S-53 与 S-54/S-55 各自独立）。
                const unsigned want = selected ? cmd.indicatorSelectedColor
                                               : cmd.indicatorNormalColor;
                const InstanceHandle shared = EnsureSharedBrush(cmd.indicatorNormalColor);
                bool wroteElem = false;
                bool wroteBrush = false;
                if (!selected && shared) {
                    wroteElem = SetBrushProp(e.handle, L"Fill", shared);
                    if (wroteElem) ++nShared;
                } else {
                    wroteElem = SetColorProp(e.handle, L"Fill", want);
                    wroteBrush = SetBrushColorOfProp(e.handle, L"Fill", want);
                }

                // 写完必须回读。SetProperty 返回 S_OK 只代表"写进去了"，
                // 不代表渲染层取的是这个值（v1.1.3 的根因就是写进被压住的层）。
                InstanceHandle bhNow = BrushHandleOfProp(e.handle, L"Fill");
                unsigned nowCol = 0;
                bool rbOk = bhNow && ReadColorOfBrush(bhNow, &nowCol);
                int nAll = 0;
                if (selected && (!rbOk || nowCol != want)) {
                    // 选中态 Fill 有 3 层，首层不一定是渲染取用的那层：逐层都染。
                    // 只对选中态做——正常态画刷被数十处共享，逐层染会波及别的元素。
                    nAll = SetAllBrushColorsOfProp(e.handle, L"Fill", want);
                    rbOk = bhNow && ReadColorOfBrush(bhNow, &nowCol);
                }
                if (wroteElem || wroteBrush || nAll) {
                    ++nInd;
                    LogW(L"[Ind] h=0x%llX w=%.1f %s b=0x%llX -> %08X (elem=%d brush=%d all=%d rb=%08X ok=%d)",
                         (unsigned long long)e.handle, w,
                         selected ? L"选中" : L"正常",
                         (unsigned long long)(selected ? 0 : shared),
                         want, (int)wroteElem, (int)wroteBrush, nAll,
                         nowCol, (int)rbOk);
                } else if (g_indFailBudget > 0) {
                    // 元素在树里、但两条写入路径都失败：Fill 索引取不到 / 画刷句柄取不到 /
                    // SetProperty 被拒。打出来才知道该补哪条兜底路径。
                    --g_indFailBudget;
                    unsigned int fillIdx = 0;
                    const bool fillFound = IndexFromChain(e.handle, L"Fill", &fillIdx);
                    const bool brushFound = BrushHandleOfProp(e.handle, L"Fill") != 0;
                    LogW(L"[IndFail] h=0x%llX w=%.1f fillFound=%d idx=%u brushFound=%d",
                         (unsigned long long)e.handle, w,
                         (int)fillFound, fillIdx, (int)brushFound);
                }
            } else if (g_activeMask & kMaskIndicator) {
                if (selected) {
                    if (g_restore.indSelected &&
                        SetBrushColorOfProp(e.handle, L"Fill", g_restore.indSelected)) ++nUndo;
                } else if (g_restore.indNormal) {
                    if (SetColorProp(e.handle, L"Fill", g_restore.indNormal)) ++nUndo;
                    // 正常态也要把共享画刷改回原色：它可能被上面的双写染过，
                    // 不还原的话「下次选中→取消选中」会闪一下目标色。
                    if (SetBrushColorOfProp(e.handle, L"Fill", g_restore.indNormal)) ++nUndo;
                }
            }
        }
    }
    // 共享画刷状态：只在变化时打一行，用来证明"所有正常态指示条指向同一个对象"。
    if (mask & kMaskIndicator) {
        static InstanceHandle lastB = 0;
        static unsigned lastC = 0;
        static int lastN = -1;
        if (g_sharedIndBrush != lastB || g_sharedIndColor != lastC || nShared != lastN) {
            lastB = g_sharedIndBrush; lastC = g_sharedIndColor; lastN = nShared;
            LogW(L"[Shared] b=0x%llX color=%08X assigned=%d",
                 (unsigned long long)g_sharedIndBrush, g_sharedIndColor, nShared);
        }
    }

    g_activeMask = mask;
    if (nTime + nDate + nTray + nInd > 0) InterlockedExchange(&g_touched, 1);
    LogW(L"[Apply] seq=%ld mask=%X time=%d date=%d tray=%d ind=%d undo=%d",
         (long)cmd.seq, mask, nTime, nDate, nTray, nInd, nUndo);
    InterlockedExchange(&g_applied, cmd.seq);
    InterlockedExchange(&g_dirty, 0);
}

static void RestoreAll() {
    if (!g_touched) {
        LogW(L"[Restore] 从未改过色，跳过（避免误清空系统视觉状态）");
        InterlockedExchange(&g_applied, 0);
        return;
    }
    auto v = Snapshot();
    int n = 0;
    for (auto& e : v) {
        if (e.name == L"TimeInnerTextBlock" || e.name == L"DateInnerTextBlock") {
            unsigned orig = (e.name == L"TimeInnerTextBlock") ? g_restore.time : g_restore.date;
            if (orig) { if (SetColorProp(e.handle, L"Foreground", orig)) ++n; }
            else if (ClearColorProp(e.handle, L"Foreground")) ++n;
        } else if (e.name == L"InnerTextBlock") {
            if (g_restore.tray) { if (SetColorProp(e.handle, L"Foreground", g_restore.tray)) ++n; }
            else if (ClearColorProp(e.handle, L"Foreground")) ++n;
        } else if (e.name == L"RunningIndicator") {
            double w = 0.0;
            // 与 ApplyColors 保持同一判据：`Width` 实测是 "Auto"，只有 `ActualWidth` 可用。
            bool selected = ReadNumber(e.handle, L"ActualWidth", &w) && w >= 10.0;
            if (selected && g_restore.indSelected) {
                // 选中态必须改回画刷自身的 Color：写元素 Fill 会被 VisualState 压住。
                // 应用时是"逐层染色"，还原也必须逐层改回，否则没被取用的那层会留色。
                if (SetAllBrushColorsOfProp(e.handle, L"Fill", g_restore.indSelected) > 0) ++n;
            } else if (!selected && g_restore.indNormal) {
                if (SetColorProp(e.handle, L"Fill", g_restore.indNormal)) ++n;
            }
        }
    }
    // 退出瞬间可能已没有任何图标处于选中态，但我们改过的选中态画刷还在树里，
    // 按缓存的句柄直接改回原色（幂等，重复写无副作用）。
    if (g_restore.indSelectedBrush && g_restore.indSelected) {
        bool ok = SetBrushColorDirect(g_restore.indSelectedBrush, g_restore.indSelected);
        unsigned back = 0;
        bool rback = ReadColorOfBrush(g_restore.indSelectedBrush, &back);
        LogW(L"[Restore] 选中态画刷 h=0x%llX -> %08X ok=%d 读回=%08X(found=%d)",
             (unsigned long long)g_restore.indSelectedBrush, g_restore.indSelected,
             (int)ok, back, (int)rback);
        if (ok) ++n;
    } else {
        LogW(L"[Restore] 选中态画刷无缓存 h=0x%llX s=%08X",
             (unsigned long long)g_restore.indSelectedBrush, g_restore.indSelected);
    }

    // 按内容兜底：选中态画刷可能被系统换成新实例，缓存的句柄会改不到它。
    // 这里对每条指示条取"当前"画刷，只要它的颜色正是我们写进去的那个，就改回原色。
    if (g_appliedIndColor) {
        for (auto& e : v) {
            if (e.name != L"RunningIndicator") continue;
            InstanceHandle bh = BrushHandleOfProp(e.handle, L"Fill");
            if (!bh) continue;
            unsigned c = 0;
            if (!ReadColorOfBrush(bh, &c) ||
                (c != g_appliedIndColor && c != g_appliedIndSelectedColor)) continue;
            double w = 0.0;
            bool sel = ReadNumber(e.handle, L"ActualWidth", &w) && w >= 10.0;
            unsigned target = (sel && g_restore.indSelected)
                                  ? g_restore.indSelected
                                  : (g_restore.indNormal ? g_restore.indNormal : 0);
            if (target && SetBrushColorDirect(bh, target)) {
                ++n;
                LogW(L"[Restore] 按内容还原 h=0x%llX %08X -> %08X",
                     (unsigned long long)bh, c, target);
            }
        }
    }
    LogW(L"[Restore] restored=%d (cache t=%08X d=%08X g=%08X n=%08X s=%08X)",
         n, g_restore.time, g_restore.date, g_restore.tray,
         g_restore.indNormal, g_restore.indSelected);
    // 还原已把共享画刷从元素上换下来，句柄不要留到下一轮。
    ResetSharedBrush();
    InterlockedExchange(&g_applied, 0);
}

/// UI 线程：只读指示条的 ActualWidth，检测"选中 ↔ 正常"切换。
///
/// 为什么必须在这里读：属性读取只能回 XAML UI 线程，否则 RPC_E_WRONG_THREAD。
/// 为什么需要它：见 g_indSelState 的注释——状态切换没有可用的回调，只能轮询。
/// 只做读取与比较，真正改色仍交给 ApplyColors，避免两处写颜色逻辑。
static void PollIndicatorState() {
    auto v = Snapshot();
    std::map<InstanceHandle, bool> cur;
    bool changed = false;
    for (const auto& e : v) {
        if (e.name != L"RunningIndicator") continue;
        double w = 0.0;
        const bool sel = ReadNumber(e.handle, L"ActualWidth", &w) && w >= 10.0;
        cur[e.handle] = sel;
        auto it = g_indSelState.find(e.handle);
        if (it == g_indSelState.end() || it->second != sel) changed = true;
    }
    if (cur.size() != g_indSelState.size()) changed = true;
    g_indSelState.swap(cur);
    if (changed) {
        InterlockedExchange(&g_dirty, 1);
        InterlockedExchange(&g_urgent, 1);
    }
}

static void UiPollProc() {
    // 不 SetEvent：主循环不等轮询结束（等下一拍应用即可）。共用 g_uiDoneEvent
    // 会让"轮询先完成"提前唤醒应用等待，反而打乱 apply 的同步。
    PollIndicatorState();
}

static void UiApplyProc() {
    ApplyColors();
    if (g_uiDoneEvent) SetEvent(g_uiDoneEvent);
}

/// 退订：退位副本必须调用，否则回调仍挂在 XAML 上。
/// 实测后果：新副本 AdviseVisualTreeChange 返回 E_UNEXPECTED 并放弃改色，
/// 表现是整片不着色，且重启宿主无用（DLL 常驻、DllMain 不再执行）。
static void UiUnadviseProc() {
    bool ok = false;
    if (g_tap) {
        if (g_vts3) ok = SUCCEEDED(g_vts3->UnadviseVisualTreeChange(g_tap));
        else if (g_vts2) ok = SUCCEEDED(g_vts2->UnadviseVisualTreeChange(g_tap));
    }
    InterlockedExchange(&g_advised, 0);
    LogW(L"[Tap] 退位副本已退订 ok=%d tid=%lu", (int)ok, GetCurrentThreadId());
    if (g_uiDoneEvent) SetEvent(g_uiDoneEvent);
}

static void UiRestoreProc() {
    RestoreAll();
    if (g_uiDoneEvent) SetEvent(g_uiDoneEvent);
}

/// UI 线程上要执行的动作。原来只有 apply/restore 两态，退位退订是第三态；
/// 用 bool 已经表达不清，改枚举。poll 只读状态、发现变化才让主循环应用。
enum UiOp { UiApply = 0, UiRestore = 1, UiUnadvise = 2, UiPoll = 3 };

class DispatchedHandler final : public ABI::Windows::UI::Core::IDispatchedHandler {
public:
    explicit DispatchedHandler(UiOp op) : m_op(op) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        if (riid == __uuidof(IUnknown) ||
            riid == __uuidof(ABI::Windows::UI::Core::IDispatchedHandler) ||
            riid == __uuidof(IAgileObject)) {
            *ppv = static_cast<ABI::Windows::UI::Core::IDispatchedHandler*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return InterlockedIncrement(&m_ref); }
    ULONG STDMETHODCALLTYPE Release() override {
        ULONG r = InterlockedDecrement(&m_ref);
        if (r == 0) delete this;
        return r;
    }
    HRESULT STDMETHODCALLTYPE Invoke() override {
        switch (m_op) {
        case UiRestore:  UiRestoreProc();  break;
        case UiUnadvise: UiUnadviseProc(); break;
        case UiPoll:     UiPollProc();     break;
        case UiApply:
        default:         UiApplyProc();    break;
        }
        Release();
        return S_OK;
    }
private:
    LONG m_ref = 1;
    UiOp m_op;
};

static HRESULT PostToUi(UiOp op) {
    if (!g_diag) return E_FAIL;
    // 每投递前复位：手动重置事件不复位会导致下一次 Wait 立即返回，
    // 从而在 UI 线程真正执行前就继续往下走（v1.1 实测踩坑）。
    if (g_uiDoneEvent) ResetEvent(g_uiDoneEvent);
    IInspectable* dispInsp = nullptr;
    HRESULT hr = g_diag->GetDispatcher(&dispInsp);
    if (FAILED(hr) || !dispInsp) return hr;

    ABI::Windows::UI::Core::ICoreDispatcher* cd = nullptr;
    hr = dispInsp->QueryInterface(__uuidof(ABI::Windows::UI::Core::ICoreDispatcher), (void**)&cd);
    if (FAILED(hr) || !cd) { dispInsp->Release(); return hr; }

    DispatchedHandler* h = new (std::nothrow) DispatchedHandler(op);
    if (!h) { cd->Release(); dispInsp->Release(); return E_OUTOFMEMORY; }

    ABI::Windows::Foundation::IAsyncAction* act = nullptr;
    hr = cd->RunAsync((ABI::Windows::UI::Core::CoreDispatcherPriority)0, h, &act);
    if (act) act->Release();
    if (FAILED(hr)) h->Release();
    cd->Release();
    dispInsp->Release();
    return hr;
}

// ---------------------------------------------------------------- 会话初始化

static bool InitTap() {
    // 注意：这里运行在 worker 线程，**不能**调 AdviseVisualTreeChange
    // （非 XAML UI 线程一律返回 E_UNEXPECTED）。订阅只在 SetSite（XAML 线程）里建立一次。
    HMODULE xaml = GetModuleHandleW(L"Windows.UI.Xaml.dll");
    if (!xaml) xaml = LoadLibraryW(L"Windows.UI.Xaml.dll");
    if (!xaml) { LogW(L"[Init] 无 Windows.UI.Xaml.dll"); return false; }

    typedef HRESULT(WINAPI* PFN_Init)(LPCWSTR, DWORD, LPCWSTR, LPCWSTR, CLSID, LPCWSTR);
    PFN_Init init = (PFN_Init)GetProcAddress(xaml, "InitializeXamlDiagnosticsEx");
    if (!init) { LogW(L"[Init] 无 InitializeXamlDiagnosticsEx"); return false; }

    wchar_t self[MAX_PATH] = L"";
    GetModuleFileNameW(g_self, self, MAX_PATH);

    // 固定 CLSID，便于与其他工具区分
    static const GUID kGuid =
        {0x2c7d5b91, 0x6e14, 0x4f83, {0xa1, 0x3d, 0x7b, 0x52, 0x9c, 0x0e, 0x64, 0x8f}};
    CLSID clsid;
    memcpy(&clsid, &kGuid, sizeof(GUID));

    for (int i = 1; i <= 200; ++i) {
        wchar_t endpoint[64];
        swprintf_s(endpoint, L"VisualDiagConnection%d", i);
        HRESULT hr = init(endpoint, GetCurrentProcessId(), L"Windows.UI.Xaml.dll", self, clsid, L"");
        if (SUCCEEDED(hr)) {
            LogW(L"[Init] 席位 %s ok", endpoint);
            break;
        }
        if (hr == E_ACCESSDENIED) { LogW(L"[Init] 席位被拒 (E_ACCESSDENIED)"); return false; }
        if (i == 200) { LogW(L"[Init] 200 个端点均失败"); return false; }
    }

    if (g_advisedEvent) WaitForSingleObject(g_advisedEvent, 5000);
    if (!g_advised) { LogW(L"[Init] 未订阅成功"); return false; }
    LogW(L"[Init] 就绪 diag=%p vts3=%p", (void*)g_diag, (void*)g_vts3);
    return true;
}

// ---------------------------------------------------------------- 工作线程

static DWORD WINAPI Worker(LPVOID) {
    // 日志路径：%LOCALAPPDATA%\ShadeBar\payload.log
    wchar_t dir[MAX_PATH] = L"";
    DWORD n = GetEnvironmentVariableW(L"LOCALAPPDATA", dir, MAX_PATH);
    if (n > 0 && n < MAX_PATH) {
        wcscat_s(dir, L"\\ShadeBar");
        CreateDirectoryW(dir, nullptr);
        swprintf_s(g_logPath, L"%s\\payload.log", dir);
    }

    LogW(L"==== ShadeBar 载荷启动 build=" L"" SHADEBAR_PAYLOAD_BUILD L" pid=%lu ====", GetCurrentProcessId());

    // 打开共享内存
    HANDLE map = OpenFileMappingW(FILE_MAP_READ | FILE_MAP_WRITE, FALSE, SHADEBAR_IPC_NAME);
    if (!map) {
        LogW(L"[Init] 打不开共享内存，退出");
        return 0;
    }
    g_ipc = (ShadeBarIpc*)MapViewOfFile(map, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, sizeof(ShadeBarIpc));
    if (!g_ipc) { LogW(L"[Init] MapViewOfFile 失败"); CloseHandle(map); return 0; }
    LogW(L"[Init] IPC 就绪 magic=0x%08X ver=%u", g_ipc->magic, g_ipc->version);
    // 抢代际：后注入者接管
    g_generation = (unsigned)InterlockedIncrement((volatile LONG*)&g_ipc->reserved[1]);
    LogW(L"[Init] 代际=%u（更早的副本会自行退位）", g_generation);

    bool ready = false;
    int initAttempts = 0;
    bool wasEnabled = false;
    LONG lastBeat = -1;
    int staleLoops = 0;

    for (;;) {
        // 活跃标记：即使宿主不在、enabled=0，也持续递增。
        // 宿主注入前用它判断"这个 explorer 里是否已有活着的载荷"——
        // 已有载荷时绝不能再注入：同一 explorer 第二次订阅会撞上失效连接，
        // 表现为"一部分正常态指示条永远不亮"，只能重启 explorer 恢复。
        InterlockedIncrement((volatile LONG*)&g_ipc->reserved[6]);
        // 被更新的副本取代：不还原（还原会抹掉新副本刚写的颜色），但**必须先退订**。
        // 退订是 v1.3.1 的关键修复：旧副本不退订时，新副本的 Advise 会拿到
        // E_UNEXPECTED 并"放弃改色"，屏幕上什么都不变；而宿主重启不会重载 DLL，
        // 于是这个状态再也恢复不了 —— 正是"改色失败、重启软件也没用"的成因。
        if (g_ipc->reserved[1] != g_generation) {
            LogW(L"[Loop] 代际 %u 已被 %u 取代，退位", g_generation, g_ipc->reserved[1]);
            if (g_advised) {
                if (SUCCEEDED(PostToUi(UiUnadvise)) && g_uiDoneEvent) {
                    WaitForSingleObject(g_uiDoneEvent, 3000);
                }
            }
            return 0;
        }
        if (g_ipc->shutdown) {
            // 宿主正常退出：还原颜色后**继续待命**，绝不退出循环。
            // 曾经这里直接 break，导致线程结束后 DLL 仍驻留 explorer：
            // 后续注入命中已加载模块、DllMain 不再执行 → 载荷永久失效，
            // 表现为"重启随影后颜色再也不变"（v1.1 实测踩坑）。
            // 也不能 Unadvise：那会丢掉订阅，恢复时拿不到树。
            LogW(L"[Loop] 宿主退出，还原后转入待命");
            if (ready && g_touched) {
                if (SUCCEEDED(PostToUi(UiRestore)) && g_uiDoneEvent) {
                    WaitForSingleObject(g_uiDoneEvent, 3000);
                }
                InterlockedExchange(&g_touched, 0);
                g_restore = RestoreColors{};
            }
            g_activeMask = 0;
            g_ipc->status = SHADEBAR_STATUS_IDLE;
            Sleep(200);
            continue;
        }

        if (!ready) {
            // 曾经这里重试 60 次就 break：线程退出，但 DLL 仍驻留 explorer，
            // 后续注入命中已加载模块、DllMain 不再执行 → 永久失效，只能重启 explorer。
            // 现在改为无限重试（后段退避），让"旧副本释放 / 任务栏重建"之后能自愈。
            ++initAttempts;
            if (initAttempts == 60) {
                LogW(L"[Loop] 初始化连续失败 60 次，转为每 10 s 重试（不再永久放弃）");
            }
            ready = InitTap();
            if (ready) {
                g_ipc->status = SHADEBAR_STATUS_READY;
                initAttempts = 0;
            } else {
                g_ipc->status = SHADEBAR_STATUS_FAILED;
                Sleep(initAttempts >= 60 ? 10000 : 2000);
            }
            continue;
        }

        const LONG seq = g_ipc->seq;
        const bool enabled = g_ipc->enabled != 0;

        if (enabled) {
            // 状态事件（悬停/选中切换）可能很密，全量应用会打满 UI 线程。
            // 决策序号变化 → 立刻应用；仅"置脏" → 限制 250 ms 一次。
            static ULONGLONG lastApplyTick = 0;
            static ULONGLONG lastPollTick = 0;
            const ULONGLONG nowTick = GetTickCount64();

            // 指示条选中态轮询：状态切换没有任何可用回调，不轮询就只能等
            // 宿主下一条指令（约 2 s）。只读 ActualWidth，代价远小于一次全量应用。
            // 用户有输入时 250 ms 一拍；空闲时 1 s（没有输入就不会切窗口）。
            // 不等它完成：它只置脏/置急，下一拍主循环自会应用。
            if (g_ipc->reserved[0] & kMaskIndicator) {
                const ULONGLONG pollGap = UserActiveRecently() ? 250ULL : 1000ULL;
                if (nowTick - lastPollTick >= pollGap) {
                    PostToUi(UiPoll);
                    lastPollTick = nowTick;
                }
            }

            const bool urgent = InterlockedExchange(&g_urgent, 0) != 0;
            if (seq != g_applied || urgent || (g_dirty && nowTick - lastApplyTick >= 250)) {
                if (SUCCEEDED(PostToUi(UiApply)) && g_uiDoneEvent) {
                    WaitForSingleObject(g_uiDoneEvent, 3000);
                }
                lastApplyTick = GetTickCount64();
            }
            wasEnabled = true;
        } else if (wasEnabled || g_applied != 0) {
            if (SUCCEEDED(PostToUi(UiRestore)) && g_uiDoneEvent) {
                WaitForSingleObject(g_uiDoneEvent, 3000);
            }
            wasEnabled = false;
        }

        g_ipc->tapSeq = g_applied;

        // 心跳看门狗：宿主异常退出（被强杀 / 崩溃）时，颜色不能永远留在任务栏上。
        // 连续 5 秒读不到心跳变化即视为宿主已死，还原并退出。
        const LONG beat = g_ipc->heartbeat;
        if (beat != lastBeat) {
            lastBeat = beat;
            staleLoops = 0;
        } else if (++staleLoops > 60) {
            // 宿主消失（被强杀 / 崩溃）：还原颜色，但**不退出**。
            // 载荷一旦退出、DLL 却仍留在 explorer 中，后续注入会命中已加载模块，
            // DllMain 不再执行 —— 等于永久失效（v1.1 实测踩坑）。
            // 因此这里转为"常驻待命"：还原 → 清零缓存 → 继续等心跳恢复。
            LogW(L"[Loop] 宿主心跳超时（%d 轮），还原并转入待命", staleLoops);
            if (ready && g_touched) {
                if (SUCCEEDED(PostToUi(UiRestore)) && g_uiDoneEvent) {
                    WaitForSingleObject(g_uiDoneEvent, 3000);
                }
                InterlockedExchange(&g_touched, 0);
                g_restore = RestoreColors{};
                g_appliedIndColor = 0;
                g_appliedIndSelectedColor = 0;
                wasEnabled = false;
                // 关键：把 seq 标记为"已处理"。
                // RestoreAll 内部会把 g_applied 归零，而共享内存里的 seq 仍是最后一条，
                // 下一轮主循环看到 seq != g_applied 就会立刻重新上色 ——
                // 实测表现就是"还原 ok=1、读回也是原色，但屏幕又被刷回灰色"。
                InterlockedExchange(&g_applied, (LONG)g_ipc->seq);
            }
            lastBeat = g_ipc->heartbeat;
            staleLoops = 0;
        }

        Sleep(120);
    }

    LogW(L"==== ShadeBar 载荷退出 ====");
    return 0;
}

// ---------------------------------------------------------------- DLL 入口

BOOL APIENTRY DllMain(HMODULE hModule, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) {
        g_self = hModule;
        DisableThreadLibraryCalls(hModule);
        g_advisedEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        g_uiDoneEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        HANDLE th = CreateThread(nullptr, 0, Worker, nullptr, 0, nullptr);
        if (th) CloseHandle(th);
    }
    return TRUE;
}

STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, LPVOID* ppv) {
    if (!ppv) return E_POINTER;
    TapFactory* f = new (std::nothrow) TapFactory();
    if (!f) return E_OUTOFMEMORY;
    return f->QueryInterface(riid, ppv);
}

STDAPI DllCanUnloadNow() { return S_FALSE; }
