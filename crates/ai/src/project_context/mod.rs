cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        mod global_rules;
        pub(crate) use global_rules::GlobalRules;
    } else {
        mod dummy_global_rules;
        pub(crate) use dummy_global_rules::GlobalRules;
    }
}
pub mod model;
