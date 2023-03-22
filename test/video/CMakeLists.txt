cmake_minimum_required(VERSION 3.16)

set(CMAKE_C_COMPILER clang)
set(CMAKE_CXX_COMPILER clang++)

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

add_executable(video_test
    main.cpp
    vpx_test.cpp
    yuv_test.cpp
    ../../src/video/yuv.cpp
    ../../src/video/vpx.cpp
    )

set_target_properties(video_test PROPERTIES CXX_STANDARD 20 C_STANDARD 11)

target_include_directories(video_test
    PRIVATE
    ${boost_algorithm_SOURCE_DIR}/include
    ${boost_assert_SOURCE_DIR}/include
    ${boost_bind_SOURCE_DIR}/include
    ${boost_config_SOURCE_DIR}/include
    ${boost_container_hash_SOURCE_DIR}/include
    ${boost_core_SOURCE_DIR}/include
    ${boost_describe_SOURCE_DIR}/include
    ${boost_detail_SOURCE_DIR}/include
    ${boost_exception_SOURCE_DIR}/include
    ${boost_function_SOURCE_DIR}/include
    ${boost_integer_SOURCE_DIR}/include
    ${boost_io_SOURCE_DIR}/include
    ${boost_iterator_SOURCE_DIR}/include
    ${boost_move_SOURCE_DIR}/include
    ${boost_mp11_SOURCE_DIR}/include
    ${boost_mpl_SOURCE_DIR}/include
    ${boost_numeric_conversion_SOURCE_DIR}/include
    ${boost_preprocessor_SOURCE_DIR}/include
    ${boost_range_SOURCE_DIR}/include
    ${boost_rational_SOURCE_DIR}/include
    ${boost_smart_ptr_SOURCE_DIR}/include
    ${boost_static_assert_SOURCE_DIR}/include
    ${boost_test_SOURCE_DIR}/include
    ${boost_throw_exception_SOURCE_DIR}/include
    ${boost_type_index_SOURCE_DIR}/include
    ${boost_type_traits_SOURCE_DIR}/include
    ${boost_utility_SOURCE_DIR}/include
    ${boost_variant2_SOURCE_DIR}/include
    ${fmt_SOURCE_DIR}/include
    ${spdlog_SOURCE_DIR}/include
    ../../src
    ../../src/third_party/openh264/codec/api/svc
    ../../third_party/libvpx
    ../../third_party/libvpx/third_party/libwebm
    ../../third_party/libvpx/third_party/libyuv/include
    )

target_link_libraries(video_test
    PRIVATE
    fmt
    pthread
    ${CMAKE_SOURCE_DIR}/third_party/libvpx/${HISUI_PACKAGE}/libvpx.a
    )

add_test(NAME video COMMAND video_test)
set_tests_properties(video PROPERTIES LABELS hisui)
