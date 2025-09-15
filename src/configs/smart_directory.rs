use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[cfg_attr(
    feature = "config-schema",
    derive(schemars::JsonSchema),
    schemars(deny_unknown_fields)
)]
#[serde(default)]
pub struct SmartDirectoryConfig<'a> {
    pub use_logical_path: bool,
    pub format: &'a str,
    pub repo_root_format: &'a str,
    pub style: &'a str,
    pub repo_root_style: Option<&'a str>,
    pub before_repo_root_style: Option<&'a str>,
    pub after_repo_root_style: Option<&'a str>,
    pub disabled: bool,
    pub read_only: &'a str,
    pub read_only_style: &'a str,
    pub home_symbol: &'a str,
    pub use_os_path_sep: bool,
}

impl Default for SmartDirectoryConfig<'_> {
    fn default() -> Self {
        Self {
            use_logical_path: true,
            format: "[$path]($style)[$read_only]($read_only_style) ",
            repo_root_format: "[$before_root_path]($before_repo_root_style)[$repo_root]($repo_root_style)[$after_root_path]($after_repo_root_style)[$read_only]($read_only_style) ",
            style: "cyan bold",
            repo_root_style: None,
            before_repo_root_style: None,
            after_repo_root_style: None,
            disabled: true,
            read_only: "🔒",
            read_only_style: "red",
            home_symbol: "~",
            use_os_path_sep: true,
        }
    }
}
