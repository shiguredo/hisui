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

add_executable(util_test
    main.cpp
    interval_test.cpp
    wildcard_test.cpp
    ../../src/util/interval.cpp
    ../../src/util/wildcard.cpp
    )

set_target_properties(util_test PROPERTIES CXX_STANDARD 20 C_STANDARD 11)

target_include_directories(util_test
    PRIVATE
    ../../src
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
    ${boost_smart_ptr_SOURCE_DIR}/include
    ${boost_static_assert_SOURCE_DIR}/include
    ${boost_test_SOURCE_DIR}/include
    ${boost_throw_exception_SOURCE_DIR}/include
    ${boost_type_traits_SOURCE_DIR}/include
    ${boost_type_index_SOURCE_DIR}/include
    ${boost_utility_SOURCE_DIR}/include
    ${boost_variant2_SOURCE_DIR}/include
    )

add_test(NAME util COMMAND util_test)
set_tests_properties(util PROPERTIES LABELS hisui)
