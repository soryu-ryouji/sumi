//! OpenAPI 契约测试：固化文件（openapi.json）与代码生成 schema 必须逐字一致——
//! 改 API 后 `cargo run -- --dump-openapi > openapi.json` 重新固化，否则本测试失败。

#[test]
fn openapi_json_in_sync() {
    let generated = crate::api::build_openapi_json();
    let committed = include_str!("../../openapi.json");
    assert_eq!(
        generated, committed,
        "openapi.json 与代码不同步：运行 `cargo run -- --dump-openapi > openapi.json` 重新固化"
    );
}
