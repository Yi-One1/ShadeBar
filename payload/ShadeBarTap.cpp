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

    char utf8[6144];
    int n = WideCharToMultiByte(CP_UTF8, 0, buf, -1, utf8, sizeof(utf8) - 2, nullptr, nullptr);
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
static CRITICAL_SECTION      g_treeCs;
static INIT_ONCE             g_treeOnce = INIT_ONCE_STATIC_INIT;

static BOOL CALLBACK TreeInit_(PINIT_ONCE, PVOID, PVOID*) {
    InitializeCriticalSection(&g_treeCs);
    return TRUE;
}

static ShadeBarIpc* g_ipc = nullptr;
/* 代际令牌：同一 explorer 里可能同时驻留多个版本的载荷（升级、重复注入）。
 * 它们共享同一块 IPC，谁都能写 —— 旧副本会用旧逻辑持续覆盖新副本的结果，
 * 现象就是"改了没反应"。约定：最后注入的实例把 reserved[1] 递增并记为自己的
 * 代际；其余实例发现代际不匹配就彻底停手，且**不做还原**（还原会抹掉新副本）。 */
static unsigned g_generation = 0;
// 是否真的向任务栏写过颜色。未写过时禁止执行还原：
// RunningIndicator 的 Fill 一旦被 ClearProperty 会变全透明（v1.1 生产事故）。
static volatile LONG g_touched = 0;

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
            VisEntry e;
            e.handle = element.Handle;
            e.parent = relation.Parent;
            e.type = element.Type ? element.Type : L"";
            e.name = element.Name ? element.Name : L"";
            const std::wstring& N = e.name;
            const std::wstring& T = e.type;
            if (N == L"TimeInnerTextBlock" || N == L"DateInnerTextBlock" ||
                N == L"RunningIndicator" || N == L"Underlay" ||
                N == L"Base" || N == L"AccentOverlay" ||
                T.find(L"AdaptiveTextBlock") != std::wstring::npos) {
                g_tree.push_back(e);
                InterlockedExchange(&g_dirty, 1);
            }
        } else {
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

static TapObject* g_tap = nullptr;

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
    int nTime = 0, nDate = 0, nTray = 0, nInd = 0, nUndo = 0;

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
        } else if (e.type.find(L"AdaptiveTextBlock") != std::wstring::npos) {
            if (mask & kMaskTray) {
                if (g_restore.tray == 0) { unsigned a = 0; if (ReadBrushColor(e.handle, L"Foreground", &a)) g_restore.tray = a; }
                if (SetColorProp(e.handle, L"Foreground", cmd.trayGlyphColor)) ++nTray;
            } else if ((g_activeMask & kMaskTray) && g_restore.tray != 0) {
                if (SetColorProp(e.handle, L"Foreground", g_restore.tray)) ++nUndo;
            }
        } else if (e.name == L"RunningIndicator") {
            // Width 实测多为 "Auto"，真实宽度在 ActualWidth：
            // 正常态约 6 px、选中态约 16 px，这是区分两态的唯一可靠依据。
            double w = 0.0;
            bool selected = ReadNumber(e.handle, L"ActualWidth", &w) && w >= 10.0;

            if (mask & kMaskIndicator) {
                g_appliedIndColor = cmd.indicatorNormalColor;

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

                // 两条路一起走，**不依赖状态判定**：
                //   写元素 Fill      -> 正常态生效（选中态会被 VisualState 的更高优先级槽压住）
                //   改 Fill 指向画刷 -> 选中态生效（正常态写入无视觉影响，但把共享画刷预染成
                //                       目标色，使「选中→正常」切换的瞬间不再闪系统色）
                // 任何一条成功都算改到了，避免因 ActualWidth 判定偏差导致整条漏改。
                bool wroteElem = SetColorProp(e.handle, L"Fill", cmd.indicatorNormalColor);
                bool wroteBrush = SetBrushColorOfProp(e.handle, L"Fill", cmd.indicatorNormalColor);

                // 写完必须回读。SetProperty 返回 S_OK 只代表"写进去了"，
                // 不代表渲染层取的是这个值（v1.1.3 的根因就是写进被压住的层）。
                InstanceHandle bhNow = BrushHandleOfProp(e.handle, L"Fill");
                unsigned nowCol = 0;
                bool rbOk = bhNow && ReadColorOfBrush(bhNow, &nowCol);
                int nAll = 0;
                if (selected && (!rbOk || nowCol != cmd.indicatorNormalColor)) {
                    // 选中态 Fill 有 3 层，首层不一定是渲染取用的那层：逐层都染。
                    // 只对选中态做——正常态画刷被数十处共享，逐层染会波及别的元素。
                    nAll = SetAllBrushColorsOfProp(e.handle, L"Fill", cmd.indicatorNormalColor);
                    rbOk = bhNow && ReadColorOfBrush(bhNow, &nowCol);
                }
                if (wroteElem || wroteBrush || nAll) {
                    ++nInd;
                    LogW(L"[Ind] h=0x%llX w=%.1f %s -> %08X (elem=%d brush=%d all=%d rb=%08X ok=%d)",
                         (unsigned long long)e.handle, w,
                         selected ? L"选中" : L"正常",
                         cmd.indicatorNormalColor, (int)wroteElem, (int)wroteBrush, nAll,
                         nowCol, (int)rbOk);
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
        } else if (e.type.find(L"AdaptiveTextBlock") != std::wstring::npos) {
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
            if (!ReadColorOfBrush(bh, &c) || c != g_appliedIndColor) continue;
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
    InterlockedExchange(&g_applied, 0);
}

static void UiApplyProc() {
    ApplyColors();
    if (g_uiDoneEvent) SetEvent(g_uiDoneEvent);
}

static void UiRestoreProc() {
    RestoreAll();
    if (g_uiDoneEvent) SetEvent(g_uiDoneEvent);
}

class DispatchedHandler final : public ABI::Windows::UI::Core::IDispatchedHandler {
public:
    explicit DispatchedHandler(bool restore) : m_restore(restore) {}
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
        if (m_restore) UiRestoreProc(); else UiApplyProc();
        Release();
        return S_OK;
    }
private:
    LONG m_ref = 1;
    bool m_restore;
};

static HRESULT PostToUi(bool restore) {
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

    DispatchedHandler* h = new (std::nothrow) DispatchedHandler(restore);
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
        // 被更新的副本取代：静默退出，不还原（还原会抹掉新副本刚写的颜色）
        if (g_ipc->reserved[1] != g_generation) {
            LogW(L"[Loop] 代际 %u 已被 %u 取代，退位", g_generation, g_ipc->reserved[1]);
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
                if (SUCCEEDED(PostToUi(true)) && g_uiDoneEvent) {
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
            if (++initAttempts > 60) { LogW(L"[Loop] 初始化重试耗尽"); break; }
            ready = InitTap();
            if (ready) {
                g_ipc->status = SHADEBAR_STATUS_READY;
            } else {
                g_ipc->status = SHADEBAR_STATUS_FAILED;
                Sleep(2000);
            }
            continue;
        }

        const LONG seq = g_ipc->seq;
        const bool enabled = g_ipc->enabled != 0;

        if (enabled) {
            // 状态事件（悬停/选中切换）可能很密，全量应用会打满 UI 线程。
            // 决策序号变化 → 立刻应用；仅"置脏" → 限制 250 ms 一次。
            static ULONGLONG lastApplyTick = 0;
            const ULONGLONG nowTick = GetTickCount64();
            if (seq != g_applied || (g_dirty && nowTick - lastApplyTick >= 250)) {
                if (SUCCEEDED(PostToUi(false)) && g_uiDoneEvent) {
                    WaitForSingleObject(g_uiDoneEvent, 3000);
                }
                lastApplyTick = GetTickCount64();
            }
            wasEnabled = true;
        } else if (wasEnabled || g_applied != 0) {
            if (SUCCEEDED(PostToUi(true)) && g_uiDoneEvent) {
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
                if (SUCCEEDED(PostToUi(true)) && g_uiDoneEvent) {
                    WaitForSingleObject(g_uiDoneEvent, 3000);
                }
                InterlockedExchange(&g_touched, 0);
                g_restore = RestoreColors{};
                g_appliedIndColor = 0;
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
