#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::all)]

// Docs.rs 向けのビルドではバインディングファイルは生成されないので include もしない
#[cfg(not(feature = "docs-rs"))]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
