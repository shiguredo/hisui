def _impl(repository_ctx):
    if not ('CLANG_VERSION' in repository_ctx.os.environ and
        'BAZEL_LLVM_DIR' in repository_ctx.os.environ):
        return

    clang_version = repository_ctx.os.environ['CLANG_VERSION']
    llvm_dir = repository_ctx.os.environ['BAZEL_LLVM_DIR']
    sysroot = repository_ctx.os.environ['BAZEL_SYSROOT'] if 'BAZEL_SYSROOT' in repository_ctx.os.environ else ''
    repository_ctx.template(
        "BUILD",
        repository_ctx.attr.src,
        {
            "%{clang_version}": clang_version,
            "%{llvm_dir}": llvm_dir,
            "%{sysroot}": sysroot,
        },
        False
    )


webrtc_clang_toolchain_configure = repository_rule(
    implementation = _impl,
    attrs = {
        "src": attr.label(executable = False, mandatory = True),
    }
)
