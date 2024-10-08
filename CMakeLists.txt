cmake_minimum_required(VERSION 3.16)

# Only interpret if() arguments as variables or keywords when unquoted.
cmake_policy(SET CMP0054 NEW)
# MSVC runtime library flags are selected by an abstraction.
cmake_policy(SET CMP0091 NEW)

include(cmake/env.cmake)

if(DEFINED CMAKE_BUILD_TYPE)
    if(${CMAKE_BUILD_TYPE} STREQUAL "Debug")
        set(SVT_AV1_BUILD_TYPE "Debug")
    else()
        set(SVT_AV1_BUILD_TYPE "Release")
    endif()
else()
    set(SVT_AV1_BUILD_TYPE "Release")
endif()

set_cache_string_from_env(BOOST_VERSION 1.83.0 "Boost のバージョン")
set_cache_string_from_env(CLI11_VERSION 2.3.2 "CLI11 のバージョン")
set_cache_string_from_env(CPP_MP4_VERSION 2023.2.1 "cpp-mp4 のバージョン")
set_cache_string_from_env(FMT_VERSION 9.1.0 "fmt のバージョン")
set_cache_string_from_env(ONEVPL_VERSION 2023.3.1 "oneVPL のバージョン")
set_cache_string_from_env(OPUS_VERSION 1.4 "Opus のバージョン")
set_cache_string_from_env(SPDLOG_VERSION 1.11.0 "spdlog のバージョン")
set_cache_string_from_env(STB_VERSION 5736b15f7ea0ffb08dd38af21067c314d6a3aae9 "stb のバージョン")

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
    NAME boost_align
    GITHUB_REPOSITORY boostorg/align
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

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
    NAME boost_container
    GITHUB_REPOSITORY boostorg/container
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
    NAME boost_core
    GITHUB_REPOSITORY boostorg/core
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_describe
    GITHUB_REPOSITORY boostorg/describe
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
    NAME boost_integer
    GITHUB_REPOSITORY boostorg/integer
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_intrusive
    GITHUB_REPOSITORY boostorg/intrusive
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
    NAME boost_json
    GITHUB_REPOSITORY boostorg/json
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
    NAME boost_mp11
    GITHUB_REPOSITORY boostorg/mp11
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_rational
    GITHUB_REPOSITORY boostorg/rational
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
    NAME boost_static_assert
    GITHUB_REPOSITORY boostorg/static_assert
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
    )

CPMAddPackage(
    NAME boost_system
    GITHUB_REPOSITORY boostorg/system
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
    NAME boost_variant2
    GITHUB_REPOSITORY boostorg/variant2
    GIT_TAG boost-${BOOST_VERSION}
    DOWNLOAD_ONLY YES
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

option(USE_ONEVPL "Use oneVPL" OFF)

if(USE_ONEVPL)
    target_compile_definitions(hisui
        PRIVATE
        USE_ONEVPL
        )

    CPMAddPackage(
        NAME vpl
        GITHUB_REPOSITORY oneapi-src/oneVPL
        VERSION ${ONEVPL_VERSION}
        OPTIONS "BUILD_SHARED_LIBS Off" "BUILD_TOOLS Off" "BUILD_EXAMPLES Off" "BUILD_PREVIEW Off" "BUILD_TOOLS_ONEVPL_EXPERIMENTAL Off"
        )

    target_sources(hisui
        PRIVATE
        src/layout/vpl_video_producer.cpp
        src/muxer/vpl_video_producer.cpp
        src/video/vaapi_utils.cpp
        src/video/vaapi_utils_drm.cpp
        src/video/vpl.cpp
        src/video/vpl_encoder.cpp
        src/video/vpl_session.cpp
        )

    target_include_directories(hisui
        PRIVATE
        ${VPL_SOURCE_DIR}/api
        )

    target_link_libraries(hisui
        PRIVATE
        drm
        va
        va-drm
        VPL
        )
endif()

target_sources(hisui
    PRIVATE
    src/archive_item.cpp
    src/audio/basic_sequencer.cpp
    src/audio/buffer_opus_encoder.cpp
    src/audio/mixer.cpp
    src/audio/opus.cpp
    src/audio/opus_decoder.cpp
    src/audio/webm_source.cpp
    src/config.cpp
    src/datetime.cpp
    src/hisui.cpp
    src/layout/archive.cpp
    src/layout/av1_video_producer.cpp
    src/layout/cell.cpp
    src/layout/cell_util.cpp
    src/layout/compose.cpp
    src/layout/composer.cpp
    src/layout/grid.cpp
    src/layout/interval.cpp
    src/layout/metadata.cpp
    src/layout/openh264_video_producer.cpp
    src/layout/overlap.cpp
    src/layout/region.cpp
    src/layout/source.cpp
    src/layout/video_source.cpp
    src/layout/vpx_video_producer.cpp
    src/metadata.cpp
    src/muxer/async_webm_muxer.cpp
    src/muxer/audio_producer.cpp
    src/muxer/av1_video_producer.cpp
    src/muxer/faststart_mp4_muxer.cpp
    src/muxer/mp4_muxer.cpp
    src/muxer/multi_channel_vpx_video_producer.cpp
    src/muxer/muxer.cpp
    src/muxer/no_video_producer.cpp
    src/muxer/openh264_video_producer.cpp
    src/muxer/opus_audio_producer.cpp
    src/muxer/simple_mp4_muxer.cpp
    src/muxer/video_producer.cpp
    src/muxer/vpx_video_producer.cpp
    src/report/reporter.cpp
    src/util/file.cpp
    src/util/interval.cpp
    src/util/json.cpp
    src/util/wildcard.cpp
    src/version/version.cpp
    src/video/av1_decoder.cpp
    src/video/basic_sequencer.cpp
    src/video/buffer_av1_encoder.cpp
    src/video/buffer_openh264_encoder.cpp
    src/video/buffer_vpx_encoder.cpp
    src/video/codec_engine.cpp
    src/video/composer.cpp
    src/video/decoder.cpp
    src/video/decoder_factory.cpp
    src/video/grid_composer.cpp
    src/video/image_source.cpp
    src/video/multi_channel_sequencer.cpp
    src/video/openh264.cpp
    src/video/openh264_decoder.cpp
    src/video/openh264_handler.cpp
    src/video/parallel_grid_composer.cpp
    src/video/preserve_aspect_ratio_scaler.cpp
    src/video/scaler.cpp
    src/video/sequencer.cpp
    src/video/simple_scaler.cpp
    src/video/vpx.cpp
    src/video/vpx_decoder.cpp
    src/video/webm_source.cpp
    src/video/yuv.cpp
    src/webm/input/audio_context.cpp
    src/webm/input/context.cpp
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
    third_party/libvpx/third_party/libyuv/source/convert_argb.cc
    third_party/libvpx/third_party/libyuv/source/convert_from.cc
    third_party/libvpx/third_party/libyuv/source/video_common.cc
    third_party/libvpx/third_party/libwebm/mkvparser/mkvparser.cc
    third_party/libvpx/third_party/libwebm/mkvparser/mkvreader.cc
    third_party/libvpx/third_party/libwebm/mkvmuxer/mkvmuxer.cc
    third_party/libvpx/third_party/libwebm/mkvmuxer/mkvwriter.cc
    third_party/libvpx/third_party/libwebm/mkvmuxer/mkvmuxerutil.cc
    )

target_include_directories(hisui
    PRIVATE
    src
    ${abseil_cpp_SOURCE_DIR}/include
    ${boost_align_SOURCE_DIR}/include
    ${boost_assert_SOURCE_DIR}/include
    ${boost_config_SOURCE_DIR}/include
    ${boost_container_SOURCE_DIR}/include
    ${boost_container_hash_SOURCE_DIR}/include
    ${boost_core_SOURCE_DIR}/include
    ${boost_describe_SOURCE_DIR}/include
    ${boost_exception_SOURCE_DIR}/include
    ${boost_integer_SOURCE_DIR}/include
    ${boost_intrusive_SOURCE_DIR}/include
    ${boost_io_SOURCE_DIR}/include
    ${boost_json_SOURCE_DIR}/include
    ${boost_move_SOURCE_DIR}/include
    ${boost_mp11_SOURCE_DIR}/include
    ${boost_rational_SOURCE_DIR}/include
    ${boost_variant2_SOURCE_DIR}/include
    ${boost_smart_ptr_SOURCE_DIR}/include
    ${boost_static_assert_SOURCE_DIR}/include
    ${boost_system_SOURCE_DIR}/include
    ${boost_throw_exception_SOURCE_DIR}/include
    ${boost_type_traits_SOURCE_DIR}/include
    ${boost_utility_SOURCE_DIR}/include
    ${cli11_SOURCE_DIR}/include
    ${cpp-mp4_SOURCE_DIR}/include
    ${fmt_SOURCE_DIR}/include
    ${google_glog_SOURCE_DIR}/src
    ${gulrak_filesystem_SOURCE_DIR}
    ${opus_SOURCE_DIR}/include
    ${spdlog_SOURCE_DIR}/include
    ${stb_SOURCE_DIR}
    third_party/openh264
    third_party/progress-cpp/include
    third_party/libvpx/third_party/libyuv/include
    third_party/libvpx/third_party/libwebm
    third_party/libvpx
    third_party/SVT-AV1/Source/API
    )

set_target_properties(hisui PROPERTIES CXX_STANDARD 20 C_STANDARD 11)

target_link_libraries(hisui
    PRIVATE
    dl
    opus
    pthread
    fmt
    shiguredo-mp4
    spdlog
    ${CMAKE_SOURCE_DIR}/third_party/libvpx/${HISUI_PACKAGE}/libvpx.a
    ${CMAKE_SOURCE_DIR}/third_party/SVT-AV1/Bin/${SVT_AV1_BUILD_TYPE}/libSvtAv1Dec.a
    ${CMAKE_SOURCE_DIR}/third_party/SVT-AV1/Bin/${SVT_AV1_BUILD_TYPE}/libSvtAv1Enc.a
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
