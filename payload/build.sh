#!/usr/bin/env bash
# 随影 ShadeBar — 注入载荷构建（Git Bash，直接调 cl.exe，绕开 vcvars/reg.exe）
#
# 产物：payload/build/ShadeBarTap.dll
# 约束：静态 CRT、无 VCRUNTIME 依赖、x64、体积尽量小
set -u

M="C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/MSVC/14.44.35207"
S="C:/Program Files (x86)/Windows Kits/10"
V="10.0.26100.0"

export MSYS2_ARG_CONV_EXCL='*'
export MSYS_NO_PATHCONV=1
CL="$M/bin/Hostx64/x64/cl.exe"
export PATH="$M/bin/Hostx64/x64:$PATH"
export INCLUDE="$M/include;$S/Include/$V/ucrt;$S/Include/$V/shared;$S/Include/$V/um;$S/Include/$V/winrt"
export LIB="$M/lib/x64;$S/Lib/$V/ucrt/x64;$S/Lib/$V/um/x64"

cd "$(dirname "$0")" || exit 1
mkdir -p build
cd build

echo "=== 编译 ShadeBarTap.dll ==="
"$CL" -nologo -utf-8 -LD -MT -O1 -Os -std:c++17 -EHsc -GS- \
      -DUNICODE -D_UNICODE -DWIN32_LEAN_AND_MEAN \
      -Fd:ShadeBarTap.pdb \
      ../ShadeBarTap.cpp \
      -Fe:ShadeBarTap.dll \
      -link -DLL -DEF:../ShadeBarTap.def -RELEASE \
      ole32.lib oleaut32.lib uuid.lib
rc=$?
if [ $rc -ne 0 ]; then echo "[x] 编译失败 rc=$rc"; exit 2; fi

echo "=== 结果 ==="
ls -la ShadeBarTap.dll
echo "=== 依赖检查（不得出现 VCRUNTIME/MSVCP）==="
strings ShadeBarTap.dll 2>/dev/null | grep -iE "^(VCRUNTIME|MSVCP)[0-9]*\.dll$" | sort -u || true
echo "OK"
