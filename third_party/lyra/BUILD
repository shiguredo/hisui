load("cc_static_library.bzl", "cc_static_library")

cc_library(
    name = "lyrac",
    srcs = [
        "lyra.cc",
    ],
    defines = [
        # "LYRA_EXPORTS",
    ],
    deps = [
        "@lyra//lyra:lyra_decoder",
    ],
    hdrs = ["lyra.h"],
    copts = ["-Ilyra"],
)

cc_static_library(
    name = "lyra",
    deps = [
        ":lyrac",
    ],
)
