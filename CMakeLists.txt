cmake_minimum_required(VERSION 3.16)

# Only interpret if() arguments as variables or keywords when unquoted.
cmake_policy(SET CMP0054 NEW)
# MSVC runtime library flags are selected by an abstraction.
cmake_policy(SET CMP0091 NEW)

include(cmake/env.cmake)

set_cache_string_from_env(BOOST_VERSION 1.75.0 "Boost のバージョン")
set_cache_string_from_env(CLI11_VERSION 1.9.1 "CLI11 のバージョン")
set_cache_string_from_env(CPP_MP4_VERSION 2021.1 "CLI11 のバージョン")
set_cache_string_from_env(FMT_VERSION 7.1.3 "fmt のバージョン")
set_cache_string_from_env(OPUS_VERSION 1.3.1 "Opus のバージョン")
set_cache_string_from_env(SPDLOG_VERSION 1.8.2 "spdlog のバージョン")
set_cache_string_from_env(STB_VERSION b42009b3b9d4ca35bc703f5310eedc74f584be58 "stb のバージョン")

set(CMAKE_C_COMPILER clang)
set(CMAKE_CXX_COMPILER clang++)

add_compile_options(
    -Wall
    -Wextra
    -Wshadow
    -Wnon-virtual-dtor
    -Wold-style-cast
    -Wcast-align
    -Wunused
    -Woverloaded-virtual
    -Wpedantic
    -Wconversion
    -Wsign-conversion
    -Wnull-dereference
    -Wdouble-promotion
    -Wformat=2
    )

set(CMAKE_CXX_FLAGS_NATIVE "-O3 -DNDEBUG -march=native -mtune=native")
set(CMAKE_C_FLAGS_NATIVE "-O3 -DNDEBUG -march=native -mtune=native")

set(CMAKE_CXX_FLAGS_PROFILE "-pg -O3 -DNDEBUG -march=native -mtune=native")
set(CMAKE_C_FLAGS_PROFILE "-pg -O3 -DNDEBUG -march=native -mtune=native")

include(cmake/tools.cmake)

CPMAddPackage(
    NAME boost_assert
    GITHUB_REPOSITORY boostorg/assert
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_config
    GITHUB_REPOSITORY boostorg/config
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_core
    GITHUB_REPOSITORY boostorg/core
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_integer
    GITHUB_REPOSITORY boostorg/integer
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_json
    GITHUB_REPOSITORY boostorg/json
    GIT_TAG boost-${BOOST_VERSION}
    OPTIONS "BOOST_JSON_STANDALONE On"
    )

CPMAddPackage(
    NAME boost_rational
    GITHUB_REPOSITORY boostorg/rational
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_static_assert
    GITHUB_REPOSITORY boostorg/static_assert
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_type_traits
    GITHUB_REPOSITORY boostorg/type_traits
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_throw_exception
    GITHUB_REPOSITORY boostorg/throw_exception
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_utility
    GITHUB_REPOSITORY boostorg/utility
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME cli11
    URL  https://github.com/CLIUtils/CLI11/archive/v${CLI11_VERSION}.zip
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME cpp-mp4
    GITHUB_REPOSITORY shiguredo/cpp-mp4
    GIT_TAG ${CPP_MP4_VERSION}
    OPTIONS "WITH_CLI Off" "WITH_EXAMPLE Off" "WITH_TEST Off"
    )

CPMAddPackage(
    NAME fmt
    GITHUB_REPOSITORY fmtlib/fmt
    GIT_TAG ${FMT_VERSION}
    )

CPMAddPackage(
    NAME opus
    GITHUB_REPOSITORY xiph/opus
    VERSION ${OPUS_VERSION}
    )

CPMAddPackage(
    NAME spdlog
    GITHUB_REPOSITORY gabime/spdlog
    VERSION ${SPDLOG_VERSION}
    )

CPMAddPackage(
    NAME stb
    GITHUB_REPOSITORY nothings/stb
    GIT_TAG ${STB_VERSION}
    DOWNLOAD_ONLY YES
    )


project(hisui C CXX)

add_executable(hisui)

option(USE_FDK_AAC "Use fdk-aac" OFF)

if(USE_FDK_AAC)
    target_compile_definitions(hisui
        PRIVATE
        USE_FDK_AAC
        )

    target_link_libraries(hisui
        PRIVATE
        fdk-aac
        m
        )

    target_sources(hisui
        PRIVATE
        src/audio/buffer_fdk_aac_encoder.cpp
        src/audio/fdk_aac.cpp
        src/muxer/fdk_aac_audio_producer.cpp
        )
endif()

target_sources(hisui
    PRIVATE
    src/config.cpp
    src/hisui.cpp
    src/metadata.cpp
    src/audio/buffer_opus_encoder.cpp
    src/audio/mixer.cpp
    src/audio/opus_decoder.cpp
    src/audio/opus.cpp
    src/audio/basic_sequencer.cpp
    src/audio/webm_source.cpp
    src/muxer/async_webm_muxer.cpp
    src/muxer/audio_producer.cpp
    src/muxer/mp4_muxer.cpp
    src/muxer/opus_audio_producer.cpp
    src/muxer/simple_mp4_muxer.cpp
    src/muxer/faststart_mp4_muxer.cpp
    src/muxer/muxer.cpp
    src/muxer/no_video_producer.cpp
    src/muxer/video_producer.cpp
    src/muxer/vpx_video_producer.cpp
    src/util/interval.cpp
    src/video/buffer_vpx_encoder.cpp
    src/video/composer.cpp
    src/video/decoder.cpp
    src/video/grid_composer.cpp
    src/video/openh264.cpp
    src/video/openh264_decoder.cpp
    src/video/openh264_handler.cpp
    src/video/parallel_grid_composer.cpp
    src/video/preserve_aspect_ratio_scaler.cpp
    src/video/scaler.cpp
    src/video/basic_sequencer.cpp
    src/video/sequencer.cpp
    src/video/simple_scaler.cpp
    src/video/image_source.cpp
    src/video/webm_source.cpp
    src/video/vpx.cpp
    src/video/vpx_decoder.cpp
    src/video/yuv.cpp
    src/webm/input/context.cpp
    src/webm/input/audio_context.cpp
    src/webm/input/video_context.cpp
    src/webm/output/context.cpp
    third_party/libvpx/third_party/libyuv/source/cpu_id.cc
    third_party/libvpx/third_party/libyuv/source/planar_functions.cc
    third_party/libvpx/third_party/libyuv/source/row_any.cc
    third_party/libvpx/third_party/libyuv/source/row_common.cc
    third_party/libvpx/third_party/libyuv/source/row_gcc.cc
    third_party/libvpx/third_party/libyuv/source/row_msa.cc
    third_party/libvpx/third_party/libyuv/source/row_neon.cc
    third_party/libvpx/third_party/libyuv/source/row_neon64.cc
    third_party/libvpx/third_party/libyuv/source/row_win.cc
    third_party/libvpx/third_party/libyuv/source/scale.cc
    third_party/libvpx/third_party/libyuv/source/scale_any.cc
    third_party/libvpx/third_party/libyuv/source/scale_common.cc
    third_party/libvpx/third_party/libyuv/source/scale_gcc.cc
    third_party/libvpx/third_party/libyuv/source/scale_msa.cc
    third_party/libvpx/third_party/libyuv/source/scale_neon.cc
    third_party/libvpx/third_party/libyuv/source/scale_neon64.cc
    third_party/libvpx/third_party/libyuv/source/scale_win.cc
    third_party/libvpx/third_party/libyuv/source/convert.cc
    third_party/libvpx/third_party/libwebm/mkvparser/mkvparser.cc
    third_party/libvpx/third_party/libwebm/mkvparser/mkvreader.cc
    third_party/libvpx/third_party/libwebm/mkvmuxer/mkvmuxer.cc
    third_party/libvpx/third_party/libwebm/mkvmuxer/mkvwriter.cc
    third_party/libvpx/third_party/libwebm/mkvmuxer/mkvmuxerutil.cc
    )

target_include_directories(hisui
    PRIVATE
    src
    ${boost_assert_SOURCE_DIR}/include
    ${boost_config_SOURCE_DIR}/include
    ${boost_core_SOURCE_DIR}/include
    ${boost_integer_SOURCE_DIR}/include
    ${boost_json_SOURCE_DIR}/include
    ${boost_rational_SOURCE_DIR}/include
    ${boost_static_assert_SOURCE_DIR}/include
    ${boost_throw_exception_SOURCE_DIR}/include
    ${boost_type_traits_SOURCE_DIR}/include
    ${boost_utility_SOURCE_DIR}/include
    ${cli11_SOURCE_DIR}/include
    ${cpp-mp4_SOURCE_DIR}/include
    ${fmt_SOURCE_DIR}/include
    ${opus_SOURCE_DIR}/include
    ${spdlog_SOURCE_DIR}/include
    ${stb_SOURCE_DIR}
    third_party/openh264
    third_party/progress-cpp/include
    third_party/libvpx/third_party/libyuv/include
    third_party/libvpx/third_party/libwebm
    third_party/libvpx
    )

set_target_properties(hisui PROPERTIES CXX_STANDARD 20 C_STANDARD 11)

target_link_libraries(hisui
    PRIVATE
    Boost::json
    dl
    opus
    pthread
    fmt
    shiguredo-mp4
    spdlog
    ${CMAKE_SOURCE_DIR}/third_party/libvpx/libvpx.a
    )

if(WITH_TEST)
    enable_testing()

    CPMAddPackage(
        NAME boost_algorithm
        GITHUB_REPOSITORY boostorg/algorithm
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_bind
        GITHUB_REPOSITORY boostorg/bind
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_container_hash
        GITHUB_REPOSITORY boostorg/container_hash
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_detail
        GITHUB_REPOSITORY boostorg/detail
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_exception
        GITHUB_REPOSITORY boostorg/exception
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_function
        GITHUB_REPOSITORY boostorg/function
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_io
        GITHUB_REPOSITORY boostorg/io
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_iterator
        GITHUB_REPOSITORY boostorg/iterator
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_move
        GITHUB_REPOSITORY boostorg/move
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_mpl
        GITHUB_REPOSITORY boostorg/mpl
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_numeric_conversion
        GITHUB_REPOSITORY boostorg/numeric_conversion
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_preprocessor
        GITHUB_REPOSITORY boostorg/preprocessor
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_range
        GITHUB_REPOSITORY boostorg/range
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_smart_ptr
        GITHUB_REPOSITORY boostorg/smart_ptr
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_test
        GITHUB_REPOSITORY boostorg/test
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    CPMAddPackage(
        NAME boost_type_index
        GITHUB_REPOSITORY boostorg/type_index
        GIT_TAG boost-${BOOST_VERSION}
        DOWNLOAD_ONLY YES
        )

    add_subdirectory(test)
endif()
