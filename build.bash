#!/usr/bin/env bash

PROGRAM="$0"

_PACKAGES=("ubuntu-20.04_x86_64")

function show_help() {
  echo "$PROGRAM [--clean] [--use-ccache] [--use-fdk-aac] [--with-test] [--build-type-native] [--package] <package>"
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
    exit 0
fi


[ -d third_party ] || mkdir third_party
cd third_party || exit 1

if [ -d libvpx ] ; then
    cd libvpx || exit 1
    patch -p1 -R < ../libwebm.patch || echo "reverse patch failed"
    git checkout master
    git pull
else
    git clone https://chromium.googlesource.com/webm/libvpx
    cd libvpx || exit 1
fi
git checkout v"${LIBVPX_VERSION}"
patch -p1 < ../libwebm.patch

libvpx_configure_options=('--disable-examples' '--disable-tools' '--disable-docs' '--disable-unit-tests' )
if [ "${BUILD_TYPE}" = "Native" ]; then
    libvpx_configure_options+=('--cpu=native')
fi

CXX="$CXX" CC="$CC" ./configure "${libvpx_configure_options[@]}"
make

cd ../..
if [ "${BUILD_TYPE}" = "Native" ]; then
    [ -d native ] || mkdir native
    cd native || exit 1
    CMAKE_FLAGS+=("-DCMAKE_BUILD_TYPE=${BUILD_TYPE}")
else
    [ -d release ] || mkdir release
    cd release || exit 1
fi
cmake  .. "${CMAKE_FLAGS[@]}"
cmake --build .

if [ $FLAG_PACKAGE -eq 1 ]; then 
    tar cvf hisui-"${HISUI_VERSION}"_ubuntu-20.04_x86_64.tar.gz hisui -C .. LICENSE NOTICE.md
fi
