pub(crate) mod compile;
pub(crate) mod helpers;
pub(crate) mod info;
pub(crate) mod install;
pub(crate) mod list;
pub(crate) mod remove;

pub(crate) use compile::compile_plugin;
pub(crate) use info::plugin_info;
pub(crate) use install::install_plugin;
pub(crate) use list::list_plugins;
pub(crate) use remove::remove_plugin;
