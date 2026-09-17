//! OpenAPI 契约测试：固化文件（openapi.json）与代码生成 schema 必须逐字一致——
//! 改 API 后 `cargo run -- --dump-openapi > openapi.json` 重新固化，否则本测试失败。

#[test]
fn openapi_json_in_sync() {
    let generated = crate::api::build_openapi_json();
    // 换行归一：Windows autocrlf 检出的 CRLF 与生成端 LF 差异不代表契约漂移
    let committed = include_str!("../../openapi.json").replace("\r\n", "\n");
    assert_eq!(
        generated, committed,
        "openapi.json 与代码不同步：运行 `cargo run -- --dump-openapi > openapi.json` 重新固化"
    );
}
