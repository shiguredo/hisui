#!/usr/bin/env bash

PROGRAM="$0"

_PACKAGES=" \
  ubuntu-20.04_x86_64 \
"

function show_help() {
  echo "$PROGRAM [--clean] [--use-ccache] [--without-test] [--build-type-native] [--package] <package>"
  echo "<package>:"
  for package in $_PACKAGES; do
    echo "  - $package"
  done
}

PACKAGE=""
FLAG_CLEAN=0
FLAG_PACKAGE=0
CMAKE_FLAGS=()
BUILD_TYPE='Release'
CXX='/usr/bin/clang++'
CC='/usr/bin/clang'

GIT='/usr/bin/git'
CMAKE='/usr/bin/cmake'
MAKE='/usr/bin/make'
MKDIR='/bin/mkdir'
RM='/bin/rm'
TAR='/bin/tar'

while [ $# -ne 0 ]; do
  case "$1" in
    "--clean" )
        FLAG_CLEAN=1
        ;;
    "--package" )
        FLAG_PACKAGE=1
        ;;
    "--without-test" )
        CMAKE_FLAGS+=('-DWITHOUT_TEST=On')
        ;;
    "--use-ccache" )
        CMAKE_FLAGS+=('-DUSE_CCACHE=On')
        CXX='/usr/bin/ccache /usr/bin/clang++'
        CC='/usr/bin/ccache /usr/bin/clang'
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
for package in $_PACKAGES; do
  if [ "$PACKAGE" = "$package" ]; then
    _FOUND=1
    break
  fi
done

if [ $_FOUND -eq 0 ]; then
  show_help
  exit 1
fi

echo "--clean: ${FLAG_CLEAN}"
echo "--package: ${FLAG_PACKAGE}"
echo "CMAKE_FLAGS:" "${CMAKE_FLAGS[@]}"
echo "BUILD_TYPE: ${BUILD_TYPE}"
echo "<package>: ${PACKAGE}"

set -ex

cd "$(dirname "$0")" || exit 1

source ./VERSION

if [ $FLAG_CLEAN -eq 1 ]; then
    rm -rf release
    rm -rf native
    exit 0
fi


[ -d third_party ] || ${MKDIR} third_party
cd third_party || exit 1

[ -d libvpx ] || ${GIT} clone https://chromium.googlesource.com/webm/libvpx
cd libvpx || exit 1
${GIT} checkout v"${LIBVPX_VERSION}"

libvpx_configure_options=('--disable-examples' '--disable-tools' '--disable-docs' '--disable-unit-tests' )
if [ "$BUILD_TYPE" = "Native" ]; then
    libvpx_configure_options+=('--cpu=native')
fi

CXX="$CXX" CC="$CC" ./configure "${libvpx_configure_options[@]}"
${MAKE}

cd ../..
if [ "$BUILD_TYPE" = "native" ]; then
    ${RM} -rf native
    ${MKDIR} native
    cd native || exit 1
    CMAKE_FLAGS+=("-DCMAKE_BUILD_TYPE=${BUILD_TYPE}")
else
    ${RM} -rf release
    ${MKDIR} release
    cd release || exit 1
fi
${CMAKE} .. "${CMAKE_FLAGS[@]}"
${CMAKE} --build .

if [ $FLAG_PACKAGE -eq 1 ]; then 
    ${TAR} cvf hisui-${HISUI_VERSION}_ubuntu-20.04_x86_64.tar.gz hisui -C .. LICENSE NOTICE.md
fi
