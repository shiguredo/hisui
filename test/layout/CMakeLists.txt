cmake_minimum_required(VERSION 3.16)

set(CMAKE_C_COMPILER clang)
set(CMAKE_CXX_COMPILER clang++)

if(DEFINED CMAKE_BUILD_TYPE)
    if(${CMAKE_BUILD_TYPE} STREQUAL "Debug")
        set(SVT_AV1_BUILD_TYPE "Debug")
    else()
        set(SVT_AV1_BUILD_TYPE "Release")
    endif()
else()
    set(SVT_AV1_BUILD_TYPE "Release")
endif()

add_compile_options(
    -Wall
    -Wextra
    -Wshadow
    -Wnon-virtual-dtor
    -Wunused
    -Wold-style-cast
    -Wcast-align
    -Woverloaded-virtual
    -Wconversion
    -Wsign-conversion
    -Wmisleading-indentation
    -pedantic)

add_executable(layout_test
    main.cpp
    cell_util_test.cpp
    grid_test.cpp
    overlap_test.cpp
    region_test.cpp
    source_test.cpp
    ../../src/archive_item.cpp
    ../../src/layout/archive.cpp
    ../../src/layout/cell.cpp
    ../../src/layout/cell_util.cpp
    ../../src/layout/grid.cpp
    ../../src/layout/interval.cpp
    ../../src/layout/overlap.cpp
    ../../src/layout/region.cpp
    ../../src/layout/source.cpp
    ../../src/layout/video_source.cpp
    ../../src/report/reporter.cpp
    ../../src/util/file.cpp
    ../../src/util/interval.cpp
    ../../src/util/json.cpp
    ../../src/version/version.cpp
    ../../src/video/av1_decoder.cpp
    ../../src/video/decoder.cpp
    ../../src/video/openh264.cpp
    ../../src/video/openh264_decoder.cpp
    ../../src/video/openh264_handler.cpp
    ../../src/video/preserve_aspect_ratio_scaler.cpp
    ../../src/video/scaler.cpp
    ../../src/video/vpx.cpp
    ../../src/video/vpx_decoder.cpp
    ../../src/video/webm_source.cpp
    ../../src/video/yuv.cpp
    ../../src/webm/input/context.cpp
    ../../src/webm/input/video_context.cpp
    ../../third_party/libvpx/third_party/libyuv/source/cpu_id.cc
    ../../third_party/libvpx/third_party/libyuv/source/planar_functions.cc
    ../../third_party/libvpx/third_party/libyuv/source/row_any.cc
    ../../third_party/libvpx/third_party/libyuv/source/row_common.cc
    ../../third_party/libvpx/third_party/libyuv/source/row_gcc.cc
    ../../third_party/libvpx/third_party/libyuv/source/row_msa.cc
    ../../third_party/libvpx/third_party/libyuv/source/row_neon.cc
    ../../third_party/libvpx/third_party/libyuv/source/row_neon64.cc
    ../../third_party/libvpx/third_party/libyuv/source/row_win.cc
    ../../third_party/libvpx/third_party/libyuv/source/scale.cc
    ../../third_party/libvpx/third_party/libyuv/source/scale_any.cc
    ../../third_party/libvpx/third_party/libyuv/source/scale_common.cc
    ../../third_party/libvpx/third_party/libyuv/source/scale_gcc.cc
    ../../third_party/libvpx/third_party/libyuv/source/scale_msa.cc
    ../../third_party/libvpx/third_party/libyuv/source/scale_neon.cc
    ../../third_party/libvpx/third_party/libyuv/source/scale_neon64.cc
    ../../third_party/libvpx/third_party/libyuv/source/scale_win.cc
    ../../third_party/libvpx/third_party/libyuv/source/convert.cc
    ../../third_party/libvpx/third_party/libwebm/mkvparser/mkvparser.cc
    ../../third_party/libvpx/third_party/libwebm/mkvparser/mkvreader.cc
    ../../third_party/libvpx/third_party/libwebm/mkvmuxer/mkvmuxer.cc
    ../../third_party/libvpx/third_party/libwebm/mkvmuxer/mkvwriter.cc
    ../../third_party/libvpx/third_party/libwebm/mkvmuxer/mkvmuxerutil.cc
    )

set_target_properties(layout_test PROPERTIES CXX_STANDARD 20 C_STANDARD 11)

target_include_directories(layout_test
    PRIVATE
    ../../src
    ${boost_algorithm_SOURCE_DIR}/include
    ${boost_align_SOURCE_DIR}/include
    ${boost_assert_SOURCE_DIR}/include
    ${boost_bind_SOURCE_DIR}/include
    ${boost_config_SOURCE_DIR}/include
    ${boost_container_SOURCE_DIR}/include
    ${boost_container_hash_SOURCE_DIR}/include
    ${boost_core_SOURCE_DIR}/include
    ${boost_describe_SOURCE_DIR}/include
    ${boost_detail_SOURCE_DIR}/include
    ${boost_exception_SOURCE_DIR}/include
    ${boost_function_SOURCE_DIR}/include
    ${boost_integer_SOURCE_DIR}/include
    ${boost_intrusive_SOURCE_DIR}/include
    ${boost_io_SOURCE_DIR}/include
    ${boost_iterator_SOURCE_DIR}/include
    ${boost_json_SOURCE_DIR}/include
    ${boost_rational_SOURCE_DIR}/include
    ${boost_move_SOURCE_DIR}/include
    ${boost_mp11_SOURCE_DIR}/include
    ${boost_mpl_SOURCE_DIR}/include
    ${boost_numeric_conversion_SOURCE_DIR}/include
    ${boost_preprocessor_SOURCE_DIR}/include
    ${boost_range_SOURCE_DIR}/include
    ${boost_smart_ptr_SOURCE_DIR}/include
    ${boost_static_assert_SOURCE_DIR}/include
    ${boost_system_SOURCE_DIR}/include
    ${boost_test_SOURCE_DIR}/include
    ${boost_throw_exception_SOURCE_DIR}/include
    ${boost_type_index_SOURCE_DIR}/include
    ${boost_type_traits_SOURCE_DIR}/include
    ${boost_utility_SOURCE_DIR}/include
    ${boost_variant2_SOURCE_DIR}/include
    ${cpp-mp4_SOURCE_DIR}/include
    ${fmt_SOURCE_DIR}/include
    ${spdlog_SOURCE_DIR}/include
    ../../third_party/libvpx/third_party/libyuv/include
    ../../third_party/libvpx/third_party/libwebm
    ../../third_party/libvpx
    ../../third_party/openh264
    ../../third_party/SVT-AV1/Source/API
    )

target_link_libraries(layout_test
    PRIVATE
    dl
    fmt
    shiguredo-mp4
    spdlog 
    ${CMAKE_SOURCE_DIR}/third_party/libvpx/${HISUI_PACKAGE}/libvpx.a
    ${CMAKE_SOURCE_DIR}/third_party/SVT-AV1/Bin/${SVT_AV1_BUILD_TYPE}/libSvtAv1Dec.a
    )

add_test(NAME layout COMMAND layout_test)
set_tests_properties(layout PROPERTIES LABELS hisui)
