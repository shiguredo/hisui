#!/usr/bin/env bash

PROGRAM="$0"

_PACKAGES=(
    "ubuntu-20.04_x86_64"
    "ubuntu-20.04_arm64"
    "ubuntu-22.04_x86_64"
    "ubuntu-22.04_arm64"
)

function show_help() {
  echo "$PROGRAM [--clean] [--use-ccache] [--use-fdk-aac] [--with-test] [--build-type-native] [--build-type-debug] [--package] <package>"
  echo "<package>:"
  for package in "${_PACKAGES[@]}"; do
    echo "  - $package"
  done
}

PACKAGE=""

FLAG_CLEAN=0
FLAG_PACKAGE=0
FLAG_WITH_TEST=0
FLAG_USE_CCACHE=0
FLAG_USE_FDK_AAC=0

CMAKE_FLAGS=()
BUILD_TYPE='Release'
CXX='clang++'
CC='clang'

while [ $# -ne 0 ]; do
  case "$1" in
    "--clean" )
        FLAG_CLEAN=1
        ;;
    "--package" )
        FLAG_PACKAGE=1
        ;;
    "--with-test" )
        FLAG_WITH_TEST=1
        ;;
    "--use-ccache" )
        FLAG_USE_CCACHE=1
        ;;
    "--use-fdk-aac" )
        FLAG_USE_FDK_AAC=1
        ;;
    "--build-type-native" )
        BUILD_TYPE="Native"
        ;;
    "--build-type-debug" )
        BUILD_TYPE="Debug"
        ;;
    --* )
        show_help
        exit 1
        ;;
    * )
        if [ -n "$PACKAGE" ]; then
            show_help
            exit 1
        fi
        PACKAGE="$1"
        ;;
esac
    shift 1
done

_FOUND=0
for package in "${_PACKAGES[@]}"; do
  if [ "$PACKAGE" = "$package" ]; then
    _FOUND=1
    break
  fi
done

if [ $_FOUND -eq 0 ]; then
  show_help
  exit 1
fi

if [ $FLAG_WITH_TEST -eq 1 ]; then
    CMAKE_FLAGS+=('-DWITH_TEST=YES')
else
    CMAKE_FLAGS+=('-DWITH_TEST=NO')
fi

if [ $FLAG_USE_CCACHE -eq 1 ]; then
    CMAKE_FLAGS+=('-DUSE_CCACHE=YES')
    CXX='ccache clang++'
    CC='ccache clang'
else
    CMAKE_FLAGS+=('-DUSE_CCACHE=NO')
fi

if [ $FLAG_USE_FDK_AAC -eq 1 ]; then
    CMAKE_FLAGS+=('-DUSE_FDK_AAC=YES')
else
    CMAKE_FLAGS+=('-DUSE_FDK_AAC=NO')
fi

echo "--clean: ${FLAG_CLEAN}"
echo "--package: ${FLAG_PACKAGE}"
echo "CMAKE_FLAGS:" "${CMAKE_FLAGS[@]}"
echo "BUILD_TYPE: ${BUILD_TYPE}"
echo "<package>: ${PACKAGE}"

set -ex

cd "$(dirname "$0")" || exit 1

# shellcheck disable=SC1091
source ./VERSION

if [ $FLAG_CLEAN -eq 1 ]; then
    rm -rf release
    rm -rf native
    rm -rf debug
    exit 0
fi


cd third_party || exit 1

# libvpx
if [ -d libvpx ] ; then
    cd libvpx || exit 1
    patch -p1 -R < ../libwebm.patch || echo "reverse patch failed"
    git checkout main
    git pull
else
    git clone --filter=tree:0 https://chromium.googlesource.com/webm/libvpx
    cd libvpx || exit 1
fi
git checkout v"${LIBVPX_VERSION}"
patch -p1 < ../libwebm.patch

libvpx_configure_options=('--disable-examples' '--disable-tools' '--disable-docs' '--disable-unit-tests' )
if [ "${BUILD_TYPE}" = "Native" ]; then
    libvpx_configure_options+=('--cpu=native')
fi

CMAKE_FLAGS+=("-DHISUI_PACKAGE=$PACKAGE")

case "$PACKAGE" in
  *_x86_64 )
    CMAKE_FLAGS+=("-DCMAKE_TOOLCHAIN_FILE=../../cmake/clang-x86_64-toolchain.cmake")
    CMAKE_FLAGS+=('-DUSE_ONEVPL=YES')
    ;;
  *_arm64 )
    CMAKE_FLAGS+=("-DCMAKE_TOOLCHAIN_FILE=../../cmake/clang-aarch64-toolchain.cmake")
    libvpx_configure_options+=(
        '--target=arm64-linux-gcc'
        '--extra-cflags=-isystem/usr/aarch64-linux-gnu/include'
        '--extra-cxxflags=-isystem/usr/aarch64-linux-gnu/include -isystem/usr/aarch64-linux-gnu/include/c++/10/aarch64-linux-gnu'
    )
    CC="$CC --target=aarch64-linux-gnu"
    CXX="$CXX --target=aarch64-linux-gnu"
esac

mkdir -p "$PACKAGE"
cd "$PACKAGE" || exit 1

CXX="$CXX" CC="$CC" ../configure "${libvpx_configure_options[@]}" || (cat config.log && exit 1)
make
cd ../../..

# SVT-AV1
cd third_party || exit 1
if [ -d SVT-AV1 ] ; then
    cd SVT-AV1 || exit 1
    git checkout master
    git pull
else
    git clone --filter=tree:0 https://gitlab.com/AOMediaCodec/SVT-AV1.git
    cd SVT-AV1 || exit 1
fi
git checkout v"${SVT_AV1_VERSION}"

stv_av1_configure_options=('--static' '--no-apps')
if [ "${BUILD_TYPE}" = "Native" ]; then
    SVT_AV1_BUILD_TYPE="Release"
    stv_av1_configure_options+=('--native' 'release')
elif [ "${BUILD_TYPE}" = "Release" ]; then
    SVT_AV1_BUILD_TYPE="Release"
    stv_av1_configure_options+=('release')
elif [ "${BUILD_TYPE}" = "Debug" ]; then
    SVT_AV1_BUILD_TYPE="Debug"
    stv_av1_configure_options+=('debug')
fi

case "$PACKAGE" in
  *_x86_64 )
    stv_av1_configure_options+=('-t' '../../../../cmake/clang-x86_64-toolchain.cmake')
    objcopy=/usr/bin/objcopy
    ;;
  *_arm64 )
    stv_av1_configure_options+=('-s' 'aarch64-linux-gnu' '-t' '../../../../cmake/clang-aarch64-toolchain.cmake')
    objcopy=/usr/aarch64-linux-gnu/bin/objcopy
esac
cd Build/linux || exit 1

./build.sh "${stv_av1_configure_options[@]}" || exit 1

cd ../../../..

${objcopy} --redefine-sym cpuinfo_is_initialized=local_cpuinfo_is_initialized third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Dec.a
${objcopy} --redefine-sym cpuinfo_initialize=local_cpuinfo_initialize third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Dec.a
${objcopy} --redefine-sym cpuinfo_deinitialize=local_cpuinfo_deinitialize third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Dec.a
${objcopy} --redefine-sym cpuinfo_get_core=local_cpuinfo_get_core third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Dec.a
${objcopy} --redefine-sym cpuinfo_isa=local_cpuinfo_isa third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Dec.a
${objcopy} --redefine-sym cpuinfo_x86_linux_init=local_cpuinfo_x86_linux_init third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Dec.a
${objcopy} --redefine-sym cpuinfo_x86_decode_vendor=local_cpuinfo_x86_decode_vendor third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Dec.a
${objcopy} --redefine-sym cpuinfo_x86_init_processor=local_cpuinfo_x86_init_processor third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Dec.a
${objcopy} --redefine-sym cpuinfo_x86_detect_isa=local_cpuinfo_x86_detect_isa third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Dec.a

${objcopy} --redefine-sym cpuinfo_is_initialized=local_cpuinfo_is_initialized third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Enc.a
${objcopy} --redefine-sym cpuinfo_initialize=local_cpuinfo_initialize third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Enc.a
${objcopy} --redefine-sym cpuinfo_deinitialize=local_cpuinfo_deinitialize third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Enc.a
${objcopy} --redefine-sym cpuinfo_get_core=local_cpuinfo_get_core third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Enc.a
${objcopy} --redefine-sym cpuinfo_isa=local_cpuinfo_isa third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Enc.a
${objcopy} --redefine-sym cpuinfo_x86_linux_init=local_cpuinfo_x86_linux_init third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Enc.a
${objcopy} --redefine-sym cpuinfo_x86_decode_vendor=local_cpuinfo_x86_decode_vendor third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Enc.a
${objcopy} --redefine-sym cpuinfo_x86_init_processor=local_cpuinfo_x86_init_processor third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Enc.a
${objcopy} --redefine-sym cpuinfo_x86_detect_isa=local_cpuinfo_x86_detect_isa third_party/SVT-AV1/Bin/"${SVT_AV1_BUILD_TYPE}"/libSvtAv1Enc.a

# Lyra
cd third_party/lyra || exit 1

if [ -d lyra ] ; then
    cd lyra || exit 1
    git checkout main 
    git pull
else
    git clone --filter=tree:0 https://github.com/google/lyra.git
    cd lyra || exit 1
fi
git checkout v"${LYRA_VERSION}"

cd ..

lyra_bazel_options=('-c')
if [ "${BUILD_TYPE}" = "Debug" ]; then
    lyra_bazel_options+=('dbg')
elif [ "${BUILD_TYPE}" = "Release" ]; then
    lyra_bazel_options+=('opt')
fi

case "$PACKAGE" in
  *_x86_64 )
      lyra_bazel_sysroot=''
      ;;
  *_arm64 )
      lyra_bazel_options+=('--config=jetson')
      lyra_bazel_sysroot='/usr/aarch64-linux-gnu'
esac

clang_raw_version=$(clang -v |& /usr/bin/grep version | rev | cut -d ' ' -f 1 | rev)
clang_version=$(echo "$clang_raw_version" | cut -d '-' -f 1)
llvm_version=$(echo "$clang_version" | cut -d '.' -f 1)

BAZEL_SYSROOT=${lyra_bazel_sysroot} BAZEL_LLVM_DIR=/usr/lib/llvm-${llvm_version} CLANG_VERSION=${clang_version} USE_BAZEL_VERSION=5.4.1 bazelisk build "${lyra_bazel_options[@]}" :lyra || exit 1
# chmod 755 bazel-bin/liblyra.a
# objcopy --redefine-sym cpuinfo_is_initialized=local_cpuinfo_is_initialized bazel-bin/liblyra.a
# objcopy --redefine-sym cpuinfo_initialize=local_cpuinfo_initialize bazel-bin/liblyra.a
# objcopy --redefine-sym cpuinfo_deinitialize=local_cpuinfo_deinitialize bazel-bin/liblyra.a

cd ../..

if [ "${BUILD_TYPE}" = "Native" ]; then
    mkdir -p "native/$PACKAGE"
    cd "native/$PACKAGE" || exit 1
    CMAKE_FLAGS+=("-DCMAKE_BUILD_TYPE=${BUILD_TYPE}")
elif [ "${BUILD_TYPE}" = "Debug" ]; then
    mkdir -p "debug/$PACKAGE"
    cd "debug/$PACKAGE" || exit 1
    CMAKE_FLAGS+=("-DCMAKE_BUILD_TYPE=${BUILD_TYPE}")
else
    mkdir -p "release/$PACKAGE"
    cd "release/$PACKAGE" || exit 1
fi
cmake  ../.. "${CMAKE_FLAGS[@]}"
cmake --build .

if [ $FLAG_PACKAGE -eq 1 ]; then 
    rm -rf "hisui-${HISUI_VERSION}"
    mkdir "hisui-${HISUI_VERSION}"
    mkdir "hisui-${HISUI_VERSION}/lyra"
    cp hisui ../../LICENSE ../../NOTICE.md "hisui-${HISUI_VERSION}"
    cp -r ../../third_party/lyra/lyra/lyra/model_coeffs "hisui-${HISUI_VERSION}/lyra"
    tar czvf "hisui-${HISUI_VERSION}_$PACKAGE.tar.gz" "hisui-${HISUI_VERSION}"
    rm -rf "hisui-${HISUI_VERSION}"
fi
