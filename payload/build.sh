#!/usr/bin/env bash
# 随影 ShadeBar — 注入载荷构建（Git Bash，直接调 cl.exe，绕开 vcvars/reg.exe）
#
# 产物：payload/build/ShadeBarTap.dll
# 约束：静态 CRT、无 VCRUNTIME 依赖、x64、体积尽量小
#
# 工具链自动探测：优先用 VSWHERE / MSVC_ROOT / SDK_ROOT 环境变量覆盖，
# 本机装在哪都能编（旧版本写死 C:\Program Files (x86)\...\14.44 那条路径，
# 换一台机器就只剩 "No such file or directory"）。
set -u

# ---- VS 安装根 ----
if [ -n "${MSVC_ROOT:-}" ]; then
    MW="$MSVC_ROOT"                                   # 形如 D:/APP/VS2026/VC/Tools/MSVC/14.52.37025
    VS_WIN="${MW%/VC/Tools/MSVC/*}"
else
    VSWHERE="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
    if [ -x "$VSWHERE" ]; then
        VS_WIN="$("$VSWHERE" -latest -products '*' -property installationPath 2>/dev/null | tr -d '\r' | head -1)"
    fi
    if [ -z "${VS_WIN:-}" ]; then
        for c in "/c/Program Files/Microsoft Visual Studio/2022/"* "/d/APP/VS2026" "/c/Program Files (x86)/Microsoft Visual Studio/2022/"*; do
            [ -d "$c/VC/Tools/MSVC" ] && VS_WIN="$c" && break
        done
    fi
    [ -z "${VS_WIN:-}" ] && { echo "[x] 找不到 Visual Studio（可设 MSVC_ROOT 指向 .../VC/Tools/MSVC/<ver>）"; exit 2; }
    VS_UNIX="$(cygpath -u "$VS_WIN" 2>/dev/null || echo "$VS_WIN")"
    MW="$VS_UNIX/VC/Tools/MSVC/$(ls "$VS_UNIX/VC/Tools/MSVC" 2>/dev/null | sort -V | tail -1)"
fi

# ---- Windows SDK ----
if [ -n "${SDK_ROOT:-}" ]; then
    S="$(cygpath -u "$SDK_ROOT" 2>/dev/null || echo "$SDK_ROOT")"
else
    S="/c/Program Files (x86)/Windows Kits/10"
fi
V="${SDK_VER:-$(ls "$S/Include" 2>/dev/null | sort -V | tail -1)}"
[ -z "${V:-}" ] && { echo "[x] 找不到 Windows SDK（可设 SDK_ROOT / SDK_VER）"; exit 2; }

# cl.exe 用 Unix 路径调用；INCLUDE/LIB 必须是 Windows 路径（分号分隔）
CL="$MW/bin/Hostx64/x64/cl.exe"
[ -x "$CL" ] || { echo "[x] 找不到 cl.exe: $CL"; exit 2; }
MW_W="$(cygpath -w "$MW" 2>/dev/null || echo "$MW")"
S_W="$(cygpath -w "$S" 2>/dev/null || echo "$S")"

export MSYS2_ARG_CONV_EXCL='*'
export MSYS_NO_PATHCONV=1
export PATH="$MW/bin/Hostx64/x64:$PATH"
export INCLUDE="$MW_W\\include;$S_W\\Include\\$V\\ucrt;$S_W\\Include\\$V\\shared;$S_W\\Include\\$V\\um;$S_W\\Include\\$V\\winrt"
export LIB="$MW_W\\lib\\x64;$S_W\\Lib\\$V\\ucrt\\x64;$S_W\\Lib\\$V\\um\\x64"

cd "$(dirname "$0")" || exit 1
mkdir -p build
cd build

echo "=== 工具链 ==="
echo "  MSVC : $MW_W"
echo "  SDK  : $S_W\\$V"

echo "=== 编译 ShadeBarTap.dll ==="
"$CL" -nologo -utf-8 -LD -MT -O1 -Os -std:c++17 -EHsc -GS- \
      -DUNICODE -D_UNICODE -DWIN32_LEAN_AND_MEAN \
      -Fd:ShadeBarTap.pdb \
      ../ShadeBarTap.cpp \
      -Fe:ShadeBarTap.dll \
      -link -DLL -DEF:../ShadeBarTap.def -RELEASE \
      ole32.lib oleaut32.lib uuid.lib user32.lib
rc=$?
if [ $rc -ne 0 ]; then echo "[x] 编译失败 rc=$rc"; exit 2; fi

echo "=== 结果 ==="
ls -la ShadeBarTap.dll
echo "=== 依赖检查（不得出现 VCRUNTIME/MSVCP）==="
DUMPBIN="$(dirname "$CL")/dumpbin.exe"
if [ -x "$DUMPBIN" ]; then
    "$DUMPBIN" //nologo //dependents ShadeBarTap.dll | grep -iE "VCRUNTIME|MSVCP" && {
        echo "[x] 出现动态 CRT 依赖"; exit 2; }
    echo "  静态 CRT，导入表仅系统 DLL"
else
    strings ShadeBarTap.dll 2>/dev/null | grep -aiE "^(VCRUNTIME|MSVCP)[0-9]*\.dll$" | sort -u || true
fi
echo "OK"
