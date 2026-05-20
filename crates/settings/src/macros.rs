//! This module defines a set of macros to standardize and simplify the process
//! of defining new settings within Warp.
//!
//! Settings are defined as enums or structs that implement [`Setting`], and are
//! organized into groups in singleton models which contain one or more settings
//! and automatically emit an event notifying interested listeners when a
//! setting changes (specifying which setting was updated).  A setting can hold
//! any type which has a default value, supports equality checks, and can be
//! both serialized and deserialized.
//!
//! # Defining settings
//!
//! ## Defining a new setting group
//!
//! This shows the simplest usage of these macros - creating a group of settings
//! where each setting has its implementation automatically generated for you.
//!
//! ```
//! # use settings::*;
//! # use settings::macros::*;
//! define_settings_group!(ExampleGroup, settings: [
//!     bool_setting: BoolSetting {
//!         type: bool,
//!         default: false,
//!         supported_platforms: SupportedPlatforms::ALL,
//!         sync_to_cloud: SyncToCloud::Never,
//!         private: false,
//!         toml_path: "example.bool_setting",
//!     },
//!     float_setting: FloatSetting {
//!         type: f32,
//!         default: 3.14,
//!         supported_platforms: SupportedPlatforms::ALL,
//!         sync_to_cloud: SyncToCloud::Never,
//!         private: false,
//!         toml_path: "example.float_setting",
//!     },
//! ]);
//! ```
//!
//! The macro also generates an `Event` type that is passed to subscribers of the
//! setting group model. The name of the type is created by appending 'ChangedEvent'
//! to the name of the setting group:
//!
//! ```
//! pub enum ExampleGroupChangedEvent {
//!   BoolSetting,
//!   FloatSetting
//! }
//! ```
//!
//! Note that this event type must be explicitly included in `use` statements to
//! bring it into scope.
//!
//! # Turning an existing enum into a setting
//!
//! You can also "upgrade" an existing enum into a setting.  As with
//! primitive-based settings, you'll need to make sure your enum implements
//! [`Default`], [`PartialEq`], [`serde::Serialize`], and
//! [`serde::Deserialize`].
//!
//! ```
//! # use schemars::JsonSchema;
//! # use serde::{Deserialize, Serialize};
//! # use settings::macros::*;
//! # use settings::*;
//! #[derive(Default, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
//! enum MyEnum {
//!     #[default]
//!     Unit,
//!     Tuple(bool),
//!     Struct { inner: f32 },
//! }
//!
//! impl settings_value::SettingsValue for MyEnum {}
//!
//! implement_setting_for_enum!(MyEnum, EnumSettingsGroup, SupportedPlatforms::ALL, SyncToCloud::Never, private: false, toml_path: "example.my_enum");
//!
//! define_settings_group!(EnumSettingsGroup, settings: [
//!     my_enum: MyEnum,
//! ]);
//! ```
//!
//! ## Syncing a setting to the cloud.
//!
//! It's easy to declare a setting as being synced to the cloud by
//! setting the sync_to_cloud field to either Global or PerPlatform.
//! For either syncing option you can specify whether the setting
//! should be synced regardless of the current state of
//! CloudPreferencesSettings.
//!
//! ```
//! # use settings::macros::*;
//! # use settings::*;
//! define_settings_group!(OverrideSettingsGroup, settings: [
//!     to_override: ToOverride {
//!         type: bool,
//!         default: false,
//!         supported_platforms: SupportedPlatforms::ALL,
//!         sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
//!         private: false,
//!         toml_path: "example.to_override",
//!     },
//! ]);
//! ```
//!
//! # Using settings
//!
//! Once you've defined a setting, usage is straightforward:
//!
//! ```
//! # use warpui::*;
//! # use settings::macros::*;
//! # use settings::manager::SettingsManager;
//! # use settings::*;
//! # use warpui_extras::user_preferences;
//! define_settings_group!(ExampleGroup, settings: [
//!     bool_setting: BoolSetting {
//!         type: bool,
//!         default: false,
//!         supported_platforms: SupportedPlatforms::ALL,
//!         sync_to_cloud: SyncToCloud::Never,
//!         private: false,
//!         toml_path: "example.bool_setting",
//!     },
//! ]);
//!
//! App::test((), |mut app| async move {
//!     // Initialize the underlying user preferences system.
//!     app.add_singleton_model(move |_ctx| {
//!         PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
//!     });
//!
//!     app.add_singleton_model(move |_ctx| {
//!         PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
//!     });
//!
//!     app.add_singleton_model(|_ctx| SettingsManager::default());
//!
//!     // Register the settings group singleton model with the application.
//!     ExampleGroup::register(&mut app);
//!
//!     // Read the value:
//!     app.read(|ctx| {
//!         let value = ExampleGroup::handle(ctx)
//!             .as_ref(ctx)
//!             .bool_setting
//!             .value();
//!     });
//!
//!     // Update the value:
//!     app.update(|ctx| {
//!         let _ = ExampleGroup::handle(ctx)
//!             .update(ctx, |example_group, ctx| {
//!                 example_group.bool_setting.set_value(true, ctx);
//!             });
//!     });
//! });
//!
//! // Subscribe to the value changes from a view:
//! impl MyView {
//!   pub fn new(ctx: &mut ViewContext<Self>) -> Self {
//!     let handle = ExampleGroup::handle(ctx);
//!     ctx.subscribe_to_model(&handle, |me, _handle, event, _ctx| {
//!       match event {
//!         ExampleGroupChangedEvent::BoolSetting { .. } => {
//!           me.handle_changed_bool_setting();
//!         }
//!       }
//!     });
//!
//!     Self {}
//!   }
//! }
//!
//! struct MyView {}
//!
//! impl Entity for MyView {
//!   type Event = ();
//! }
//!
//! impl View for MyView {
//!   fn ui_name() -> &'static str {
//!     "MyView"
//!   }
//!
//!   fn render(&self, app_ctx: &AppContext) -> Box<dyn Element> {
//!     elements::Rect::new().finish()
//!   }
//! }
//!
//! impl MyView {
//!   fn handle_changed_bool_setting(&self) {
//!     println!("Bool setting changed.");
//!   }
//! }
//! ```

pub use ::concat_idents::concat_idents;

#[macro_export]
macro_rules! define_setting {
    // Convenience arm: with storage_key + toml_path + max_table_depth
    ($name:ident: $type:ty, default: $default:tt, supported_platforms: $supported_platforms: expr, group: $group:path, storage_key: $storage_key:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, toml_path: $toml_path:expr, max_table_depth: $mtd:literal $(, description: $desc:literal)? $(, feature_flag: $flag:path)?) => {
        $crate::macros::define_setting!(@base $name: $type, default: $default, supported_platforms: $supported_platforms, group: $group, sync_to_cloud: $sync_to_cloud, private: $private, storage_key: $storage_key, toml_path_value: Some($toml_path), max_table_depth_value: $mtd $(, description: $desc)? $(, feature_flag: $flag)?);
    };
    // Convenience arm: with toml_path + max_table_depth (no explicit storage_key)
    ($name:ident: $type:ty, default: $default:tt, supported_platforms: $supported_platforms: expr, group: $group:path, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, toml_path: $toml_path:expr, max_table_depth: $mtd:literal $(, description: $desc:literal)? $(, feature_flag: $flag:path)?) => {
        $crate::macros::define_setting!(@base $name: $type, default: $default, supported_platforms: $supported_platforms, group: $group, sync_to_cloud: $sync_to_cloud, private: $private, storage_key: stringify!($name), toml_path_value: Some($toml_path), max_table_depth_value: $mtd $(, description: $desc)? $(, feature_flag: $flag)?);
    };
    // Convenience arm: with storage_key + toml_path
    ($name:ident: $type:ty, default: $default:tt, supported_platforms: $supported_platforms: expr, group: $group:path, storage_key: $storage_key:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, toml_path: $toml_path:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)?) => {
        $crate::macros::define_setting!(@base $name: $type, default: $default, supported_platforms: $supported_platforms, group: $group, sync_to_cloud: $sync_to_cloud, private: $private, storage_key: $storage_key, toml_path_value: Some($toml_path) $(, description: $desc)? $(, feature_flag: $flag)?);
    };
    // Convenience arm: with toml_path (no explicit storage_key)
    ($name:ident: $type:ty, default: $default:tt, supported_platforms: $supported_platforms: expr, group: $group:path, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, toml_path: $toml_path:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)?) => {
        $crate::macros::define_setting!(@base $name: $type, default: $default, supported_platforms: $supported_platforms, group: $group, sync_to_cloud: $sync_to_cloud, private: $private, storage_key: stringify!($name), toml_path_value: Some($toml_path) $(, description: $desc)? $(, feature_flag: $flag)?);
    };
    // Convenience arm: without toml_path (private settings with explicit storage_key)
    ($name:ident: $type:ty, default: $default:tt, supported_platforms: $supported_platforms: expr, group: $group:path, storage_key: $storage_key:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)?) => {
        $crate::macros::define_setting!(@base $name: $type, default: $default, supported_platforms: $supported_platforms, group: $group, sync_to_cloud: $sync_to_cloud, private: $private, storage_key: $storage_key, toml_path_value: None::<&str> $(, description: $desc)? $(, feature_flag: $flag)?);
    };
    // Convenience arm: without toml_path (private settings with default storage_key)
    ($name:ident: $type:ty, default: $default:tt, supported_platforms: $supported_platforms: expr, group: $group:path, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)?) => {
        $crate::macros::define_setting!(@base $name: $type, default: $default, supported_platforms: $supported_platforms, group: $group, sync_to_cloud: $sync_to_cloud, private: $private, storage_key: stringify!($name), toml_path_value: None::<&str> $(, description: $desc)? $(, feature_flag: $flag)?);
    };
    // Base arm: generates the struct and Setting impl
    (@base $name:ident: $type:ty, default: $default:tt, supported_platforms: $supported_platforms: expr, group: $group:path, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, storage_key: $storage_key:expr, toml_path_value: $toml_path_value:expr $(, max_table_depth_value: $mtd:literal)? $(, description: $desc:literal)? $(, feature_flag: $flag:path)?) => {
        pub struct $name {
            inner: $type,
            is_explicitly_set: bool,
        }

        const _: () = {
            let toml_path: Option<&str> = $toml_path_value;
            if !$private && toml_path.is_none() {
                panic!("non-private settings must specify a toml_path");
            }
        };

        impl $crate::Setting for $name {
            type Value = $type;
            type Group = $group;

            /// Creates a new setting with the given value, if provided, otherwise
            /// uses the default value. Also tracks whether the setting was explicitly
            /// set or not.
            fn new(value: Option<Self::Value>) -> Self {
                match value {
                    Some(v) => Self {
                        inner: v,
                        is_explicitly_set: true,
                    },
                    None => {
                        let default_value = Self::default_value();
                        log::debug!(
                            "Initializing {} to default value: {:?}",
                            Self::setting_name(),
                            default_value
                        );
                        Self {
                            inner: default_value,
                            is_explicitly_set: false,
                        }
                    }
                }
            }

            fn setting_name() -> &'static str {
                stringify!($name)
            }

            fn toml_path() -> Option<&'static str> {
                $toml_path_value
            }

            fn storage_key() -> &'static str {
                $storage_key
            }

            fn toml_key() -> &'static str {
                const KEY: &str = match $toml_path_value {
                    Some(path) => $crate::toml_path_storage_key(path),
                    None => $storage_key,
                };
                KEY
            }

            fn hierarchy() -> Option<&'static str> {
                const HIER: Option<&str> = match $toml_path_value {
                    Some(path) => $crate::toml_path_hierarchy(path),
                    None => None,
                };
                HIER
            }

            fn sync_to_cloud() -> $crate::SyncToCloud {
                $sync_to_cloud
            }

            fn is_private() -> bool {
                $private
            }

            fn supported_platforms() -> SupportedPlatforms {
                $supported_platforms
            }

            fn value(&self) -> &Self::Value {
                &self.inner
            }

            fn clear_value(
                &mut self,
                ctx: &mut warpui::ModelContext<Self::Group>,
            ) -> anyhow::Result<()> {
                use $crate::ChangeEventReason;
                Self::clear_from_preferences(Self::preferences_for_setting(ctx))?;
                self.inner = self.validate(Self::default_value());
                self.is_explicitly_set = false;
                ctx.emit($crate::macros::concat_idents!(EventName = $group, ChangedEvent { EventName::$name {
                    change_event_reason: ChangeEventReason::Clear,
                }}));
                Ok(())
            }

            fn set_value_from_cloud_sync(
                &mut self,
                new_value: Self::Value,
                ctx: &mut warpui::ModelContext<Self::Group>,
            ) -> anyhow::Result<()> {
                use $crate::ChangeEventReason;
                let changed_in_storage =
                    Self::write_to_preferences(&new_value, Self::preferences_for_setting(ctx))?;
                if self.value() != &new_value || changed_in_storage {
                    self.inner = self.validate(new_value);
                    self.is_explicitly_set = true;
                    ctx.emit($crate::macros::concat_idents!(EventName = $group, ChangedEvent { EventName::$name {
                        change_event_reason: ChangeEventReason::CloudSync,
                    }}));
                }
                Ok(())
            }

            fn set_value(
                &mut self,
                new_value: Self::Value,
                ctx: &mut warpui::ModelContext<Self::Group>,
            ) -> anyhow::Result<()> {
                use $crate::ChangeEventReason;
                let changed_in_storage =
                    Self::write_to_preferences(&new_value, Self::preferences_for_setting(ctx))?;
                if self.value() != &new_value || changed_in_storage {
                    self.inner = self.validate(new_value);
                    self.is_explicitly_set = true;
                    ctx.emit($crate::macros::concat_idents!(EventName = $group, ChangedEvent { EventName::$name {
                        change_event_reason: ChangeEventReason::LocalChange,
                    }}));
                }
                Ok(())
            }

            fn load_value(
                &mut self,
                new_value: Self::Value,
                explicitly_set: bool,
                ctx: &mut warpui::ModelContext<Self::Group>,
            ) -> anyhow::Result<()> {
                use $crate::ChangeEventReason;
                let validated = self.validate(new_value);
                if self.value() != &validated || self.is_explicitly_set != explicitly_set {
                    self.inner = validated;
                    self.is_explicitly_set = explicitly_set;
                    ctx.emit($crate::macros::concat_idents!(EventName = $group, ChangedEvent { EventName::$name {
                        change_event_reason: ChangeEventReason::LocalChange,
                    }}));
                }
                Ok(())
            }

            fn default_value() -> Self::Value {
                $default
            }

            fn is_supported_on_current_platform(&self) -> bool {
                $supported_platforms.matches_current_platform()
            }

            fn is_value_explicitly_set(&self) -> bool {
                self.is_explicitly_set
            }

            $(
            fn max_table_depth() -> Option<u32> {
                Some($mtd)
            }
            )?
        }

        impl std::ops::Deref for $name {
            type Target = $type;

            fn deref(&self) -> &Self::Target {
                use $crate::Setting;
                self.value()
            }
        }

        $crate::submit_schema_entry!(
            private: $private,
            description: $crate::_schema_default_description!($($desc)?),
            toml_path_value: $toml_path_value,
            fallback_storage_key: $storage_key,
            supported_platforms: $supported_platforms,
            feature_flag: $crate::_schema_default_flag!($($flag)?),
            max_table_depth: $crate::_schema_default_max_table_depth!($($mtd)?),
            default: $default,
            value_type: $type
        );
    };
}
pub use define_setting;

#[macro_export]
macro_rules! maybe_define_setting {
    // storage_key + toml_path + max_table_depth
    ($setting:ident, group: $group:path, { type: $value_type:ty, default: $default:expr, supported_platforms: $supported_platforms:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, storage_key: $key:expr, toml_path: $toml_path:expr, max_table_depth: $mtd:literal $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)? }) => {
        $crate::macros::define_setting!(
            $setting: $value_type,
            default: $default,
            supported_platforms: $supported_platforms,
            group: $group,
            storage_key: $key,
            sync_to_cloud: $sync_to_cloud,
            private: $private,
            toml_path: $toml_path,
            max_table_depth: $mtd
            $(, description: $desc)?
            $(, feature_flag: $flag)?
        );
    };
    // toml_path + max_table_depth (no explicit storage_key)
    ($setting:ident, group: $group:path, { type: $value_type:ty, default: $default:expr, supported_platforms: $supported_platforms:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, toml_path: $toml_path:expr, max_table_depth: $mtd:literal $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)? }) => {
        $crate::macros::define_setting!(
            $setting: $value_type,
            default: $default,
            supported_platforms: $supported_platforms,
            group: $group,
            sync_to_cloud: $sync_to_cloud,
            private: $private,
            toml_path: $toml_path,
            max_table_depth: $mtd
            $(, description: $desc)?
            $(, feature_flag: $flag)?
        );
    };
    // storage_key + toml_path
    ($setting:ident, group: $group:path, { type: $value_type:ty, default: $default:expr, supported_platforms: $supported_platforms:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, storage_key: $key:expr, toml_path: $toml_path:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)? }) => {
        $crate::macros::define_setting!(
            $setting: $value_type,
            default: $default,
            supported_platforms: $supported_platforms,
            group: $group,
            storage_key: $key,
            sync_to_cloud: $sync_to_cloud,
            private: $private,
            toml_path: $toml_path
            $(, description: $desc)?
            $(, feature_flag: $flag)?
        );
    };
    // toml_path only (no explicit storage_key)
    ($setting:ident, group: $group:path, { type: $value_type:ty, default: $default:expr, supported_platforms: $supported_platforms:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, toml_path: $toml_path:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)? }) => {
        $crate::macros::define_setting!(
            $setting: $value_type,
            default: $default,
            supported_platforms: $supported_platforms,
            group: $group,
            sync_to_cloud: $sync_to_cloud,
            private: $private,
            toml_path: $toml_path
            $(, description: $desc)?
            $(, feature_flag: $flag)?
        );
    };
    // storage_key only, no toml_path (private settings with custom key)
    ($setting:ident, group: $group:path, { type: $value_type:ty, default: $default:expr, supported_platforms: $supported_platforms:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr, storage_key: $key:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)? }) => {
        $crate::macros::define_setting!(
            $setting: $value_type,
            default: $default,
            supported_platforms: $supported_platforms,
            group: $group,
            storage_key: $key,
            sync_to_cloud: $sync_to_cloud,
            private: $private
            $(, description: $desc)?
            $(, feature_flag: $flag)?
        );
    };
    // neither toml_path nor storage_key (private settings with default key)
    ($setting:ident, group: $group:path, { type: $value_type:ty, default: $default:expr, supported_platforms: $supported_platforms:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)? }) => {
        $crate::macros::define_setting!(
            $setting: $value_type,
            default: $default,
            supported_platforms: $supported_platforms,
            group: $group,
            sync_to_cloud: $sync_to_cloud,
            private: $private
            $(, description: $desc)?
            $(, feature_flag: $flag)?
        );
    };
    ($setting:ident, group: $group:path) => {};
}
pub use maybe_define_setting;

#[macro_export]
macro_rules! implement_setting_for_enum {
    // Base arm with all parameters
    (@base $name:ident, $group:path, $supported_platforms:expr, $sync_to_cloud:expr, private: $private:expr, storage_key: $storage_key:expr, toml_path_value: $toml_path_value:expr $(, max_table_depth_value: $mtd:literal)? $(, description: $desc:literal)? $(, feature_flag: $flag:path)?) => {
        const _: () = {
            let toml_path: Option<&str> = $toml_path_value;
            if !$private && toml_path.is_none() {
                panic!("non-private settings must specify a toml_path");
            }
        };

        impl $crate::Setting for $name {
            type Value = $name;
            type Group = $group;

            fn new(value: Option<Self::Value>) -> Self {
                match value {
                    Some(v) => v,
                    None => Self::default_value()
                }
            }

            fn setting_name() -> &'static str {
                stringify!($name)
            }

            fn toml_path() -> Option<&'static str> {
                $toml_path_value
            }

            fn storage_key() -> &'static str {
                $storage_key
            }

            fn toml_key() -> &'static str {
                const KEY: &str = match $toml_path_value {
                    Some(path) => $crate::toml_path_storage_key(path),
                    None => $storage_key,
                };
                KEY
            }

            fn hierarchy() -> Option<&'static str> {
                const HIER: Option<&str> = match $toml_path_value {
                    Some(path) => $crate::toml_path_hierarchy(path),
                    None => None,
                };
                HIER
            }

            fn sync_to_cloud() -> $crate::SyncToCloud {
                $sync_to_cloud
            }

            fn is_private() -> bool {
                $private
            }

            fn supported_platforms() -> SupportedPlatforms {
                $supported_platforms
            }

            fn value(&self) -> &Self::Value {
                &self
            }

            fn clear_value(
                &mut self,
                ctx: &mut warpui::ModelContext<Self::Group>,
            ) -> anyhow::Result<()> {
                use $crate::ChangeEventReason;
                Self::clear_from_preferences(Self::preferences_for_setting(ctx))?;
                *self = self.validate(Self::default_value());
                ctx.emit($crate::macros::concat_idents!(EventName = $group, ChangedEvent { EventName::$name {
                    change_event_reason: ChangeEventReason::Clear,
                }}));
                Ok(())
            }

            fn set_value_from_cloud_sync(
                &mut self,
                new_value: Self::Value,
                ctx: &mut warpui::ModelContext<Self::Group>,
            ) -> anyhow::Result<()> {
                use $crate::ChangeEventReason;
                let changed_in_storage =
                    Self::write_to_preferences(&new_value, Self::preferences_for_setting(ctx))?;
                if self.value() != &new_value || changed_in_storage {
                    *self = self.validate(new_value);
                    ctx.emit($crate::macros::concat_idents!(EventName = $group, ChangedEvent { EventName::$name {
                        change_event_reason: ChangeEventReason::CloudSync,
                    }}));
                }
                Ok(())
            }

            fn set_value(
                &mut self,
                new_value: Self::Value,
                ctx: &mut warpui::ModelContext<Self::Group>,
            ) -> anyhow::Result<()> {
                use $crate::ChangeEventReason;
                let changed_in_storage =
                    Self::write_to_preferences(&new_value, Self::preferences_for_setting(ctx))?;
                if self.value() != &new_value || changed_in_storage {
                    *self = self.validate(new_value);
                    ctx.emit($crate::macros::concat_idents!(EventName = $group, ChangedEvent { EventName::$name {
                        change_event_reason: ChangeEventReason::LocalChange,
                    }}));
                }
                Ok(())
            }

            fn load_value(
                &mut self,
                new_value: Self::Value,
                _explicitly_set: bool,
                ctx: &mut warpui::ModelContext<Self::Group>,
            ) -> anyhow::Result<()> {
                use $crate::ChangeEventReason;
                let validated = self.validate(new_value);
                if self.value() != &validated {
                    *self = validated;
                    ctx.emit($crate::macros::concat_idents!(EventName = $group, ChangedEvent { EventName::$name {
                        change_event_reason: ChangeEventReason::LocalChange,
                    }}));
                }
                Ok(())
            }

            fn default_value() -> Self::Value {
                Self::default()
            }

            fn is_supported_on_current_platform(&self) -> bool {
                $supported_platforms.matches_current_platform()
            }

            fn is_value_explicitly_set(&self) -> bool {
                // For enums using implement_setting_for_enum, we don't track explicit setting
                // TODO(advait): deprecate this in favour of struct settings in a follow-up PR.
                true
            }

            $(
            fn max_table_depth() -> Option<u32> {
                Some($mtd)
            }
            )?
        }

        $crate::submit_schema_entry!(
            private: $private,
            description: $crate::_schema_default_description!($($desc)?),
            toml_path_value: $toml_path_value,
            fallback_storage_key: $storage_key,
            supported_platforms: $supported_platforms,
            feature_flag: $crate::_schema_default_flag!($($flag)?),
            max_table_depth: $crate::_schema_default_max_table_depth!($($mtd)?),
            default: { <$name as Default>::default() },
            value_type: $name
        );
    };
    // toml_path + max_table_depth
    ($name:ident, $group:path, $supported_platforms:expr, $sync_to_cloud:expr, private: $private:expr, toml_path: $toml_path:expr, max_table_depth: $mtd:literal $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)?) => {
        $crate::macros::implement_setting_for_enum!(@base $name, $group, $supported_platforms, $sync_to_cloud, private: $private, storage_key: stringify!($name), toml_path_value: Some($toml_path), max_table_depth_value: $mtd $(, description: $desc)? $(, feature_flag: $flag)?);
    };
    // toml_path only
    ($name:ident, $group:path, $supported_platforms:expr, $sync_to_cloud:expr, private: $private:expr, toml_path: $toml_path:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)?) => {
        $crate::macros::implement_setting_for_enum!(@base $name, $group, $supported_platforms, $sync_to_cloud, private: $private, storage_key: stringify!($name), toml_path_value: Some($toml_path) $(, description: $desc)? $(, feature_flag: $flag)?);
    };
    // neither (private settings)
    ($name:ident, $group:path, $supported_platforms:expr, $sync_to_cloud:expr, private: $private:expr $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)?) => {
        $crate::macros::implement_setting_for_enum!(@base $name, $group, $supported_platforms, $sync_to_cloud, private: $private, storage_key: stringify!($name), toml_path_value: None::<&str> $(, description: $desc)? $(, feature_flag: $flag)?);
    };
}
pub use implement_setting_for_enum;

/// By defining a trait that the settings groups implement, we're able to call
/// methods like `is_supported_on_current_platform()` without knowing the exact settings
/// group we're operating on at compile time.
pub trait SettingSection {
    fn is_supported_on_current_platform(&self) -> bool;
}

#[macro_export]
macro_rules! define_settings_group {
    ($group:ident, settings: [$($var:ident: $setting:ident $({ type: $value_type:ty, default: $default:expr, supported_platforms: $supported_platforms:expr, sync_to_cloud: $sync_to_cloud:expr, private: $private:expr $(, storage_key: $storage_key:literal)? $(, toml_path: $toml_path:literal)? $(, max_table_depth: $mtd:literal)? $(, description: $desc:literal)? $(, feature_flag: $flag:path)? $(,)? })? $(,)? )*]) => {
        $(
            $crate::macros::maybe_define_setting!($setting, group: $group $(, { type: $value_type, default: $default, supported_platforms: $supported_platforms, sync_to_cloud: $sync_to_cloud, private: $private $(, storage_key: $storage_key)? $(, toml_path: $toml_path)? $(, max_table_depth: $mtd)? $(, description: $desc)? $(, feature_flag: $flag)? })?);
        )*

        pub struct $group {
            $(
                pub $var: $setting,
            )*
        }

        impl $group {
            #[allow(dead_code)]
            fn new_from_storage(ctx: &mut warpui::ModelContext<Self>) -> Self {
                use $crate::Setting;
                Self {
                    $(
                        $var: <$setting>::new_from_storage(ctx),
                    )*
                }
            }

            #[cfg(any(test, feature = "integration_tests"))]
            #[allow(dead_code)]
            pub fn new_with_defaults(_ctx: &mut warpui::ModelContext<Self>) -> Self {
                use $crate::Setting;
                Self {
                    $(
                        $var: <$setting>::new(None),
                    )*
                }
            }

            #[allow(dead_code)]
            pub fn register(ctx: &mut (impl warpui::GetSingletonModelHandle + warpui::AddSingletonModel + warpui::UpdateModel)) -> warpui::ModelHandle<Self> {
                let settings_group = ctx.add_singleton_model(|ctx| {
                    Self::new_from_storage(ctx)
                });

                // Wire up settings event update functions for all settings
                $(
                    $crate::macros::register_settings_events!(
                        $group,
                        $var,
                        $setting,
                        settings_group.clone(),
                        ctx
                    );
                )*
                settings_group
            }
        }

        impl $crate::macros::SettingSection for $group {
            /// If any of the settings in the setting group are supported, then the group is supported.
            /// If none of the settings in the group are supported, then the group is not supported.
            fn is_supported_on_current_platform(&self) -> bool {
                use $crate::Setting;
                $(
                    if self.$var.is_supported_on_current_platform() {
                        return true;
                    }
                )*
                false
            }
        }

        $crate::macros::concat_idents!(EventName = $group, ChangedEvent {
            use $crate::ChangeEventReason;
            #[derive(Debug)]
            #[allow(clippy::enum_variant_names)]
            pub enum EventName {
                $(
                    $setting {
                        #[allow(dead_code)]
                        change_event_reason: ChangeEventReason,
                    },
                )*
            }

            impl warpui::Entity for $group {
                type Event = EventName;
            }
        });

        impl warpui::SingletonEntity for $group {}
    };
}
pub use define_settings_group;

/// Registers listeners for settings events that get piped through the
/// SettingsManager. These events allow for anyone to listen to settings
/// changes based on storage key rather than individual settings models.
#[macro_export]
macro_rules! register_settings_events {
    ( $group:ident, $var:ident, $setting:ident, $handle:expr, $ctx:expr ) => {{
        $crate::macros::generate_settings_event_fn!($group, $var, $setting);

        concat_idents::concat_idents!(fn_name = register_events_for_, $setting, {
            fn_name($handle, $ctx);
        });
    }};
}
pub use register_settings_events;

/// Generates a function that can be used to register event handlers for a
/// for letting the SettingsManager know when a setting has been updated.
/// Used for managing the flow of events for local and cloud settings.
#[macro_export]
macro_rules! generate_settings_event_fn {
    ( $group:ident, $var:ident, $setting:ident ) => {
        concat_idents::concat_idents!(fn_name = register_events_for_, $setting, {
            #[allow(dead_code)]
            #[allow(non_snake_case)]
            fn fn_name(
                settings_group: warpui::ModelHandle<$group>,
                ctx: &mut (
                         impl warpui::GetSingletonModelHandle
                         + warpui::AddSingletonModel
                         + warpui::UpdateModel
                     ),
            ) {
                use anyhow::anyhow;
                use serde_json;
                use warpui::SingletonEntity;
                use $crate::Setting as _;
                use $crate::manager::{SettingsEvent, SettingsManager};
                SettingsManager::handle(ctx).update(ctx, |manager, ctx| {
                    // Propagate per settings change events through the SettingsManager
                    ctx.subscribe_to_model(&settings_group, |_manager, _, ctx| {
                        ctx.emit(SettingsEvent::LocalPreferencesUpdated {
                            storage_key: $setting::storage_key().to_string(),
                            sync_to_cloud: $setting::sync_to_cloud(),
                        });
                    });
                    // Register callbacks for updating individual settings model by storage key
                    let settings_group_update_clone = settings_group.clone();
                    let settings_group_reset_clone = settings_group.clone();
                    let settings_group_load_clone = settings_group.clone();
                    let settings_group_is_syncable_clone = settings_group.clone();
                    let serialized_default_value =
                        serde_json::to_string(&$setting::default_value())
                            .expect("default should serialize");
                    let file_serialized_default_value = {
                        use $crate::_settings_value::SettingsValue as _;
                        let file_value = $setting::default_value().to_file_value();
                        serde_json::to_string(&file_value)
                            .expect("default file value should serialize")
                    };
                    manager.register_setting(
                        $setting::storage_key(),
                        $setting::sync_to_cloud(),
                        $setting::supported_platforms(),
                        serialized_default_value,
                        file_serialized_default_value,
                        $setting::hierarchy(),
                        $setting::toml_key(),
                        $setting::max_table_depth(),
                        $setting::is_private(),
                        move |value, from_cloud_sync, ctx| {
                            use $crate::_settings_value::SettingsValue as _;
                            // Try SettingsValue first (handles snake_case enums etc.),
                            // then fall back to serde for cloud sync values.
                            let value = serde_json::from_str::<serde_json::Value>(&value)
                                .ok()
                                .and_then(|json_val| {
                                    <$setting as $crate::Setting>::Value::from_file_value(&json_val)
                                })
                                .or_else(|| serde_json::from_str(&value).ok());
                            let Some(value) = value else {
                                return Err(anyhow!(
                                    "Failed to parse updated value for setting {}: Not updating",
                                    $setting::storage_key()
                                ));
                            };
                            settings_group_update_clone.update(ctx, |settings_group, ctx| {
                                if from_cloud_sync {
                                    settings_group.$var.set_value_from_cloud_sync(value, ctx)
                                } else {
                                    settings_group.$var.set_value(value, ctx)
                                }
                            })
                        },
                        move |ctx| {
                            settings_group_reset_clone.update(ctx, |settings_group, ctx| {
                                if settings_group
                                    .$var
                                    .is_setting_syncable_on_current_platform(true)
                                {
                                    log::debug!(
                                        "Clearing cloud synced setting from local storage: {}",
                                        $setting::storage_key()
                                    );
                                    settings_group.$var.clear_value(ctx)
                                } else {
                                    Ok(())
                                }
                            })
                        },
                        move |value, explicitly_set, ctx| {
                            use $crate::_settings_value::SettingsValue as _;
                            let value = serde_json::from_str::<serde_json::Value>(&value)
                                .ok()
                                .and_then(|json_val| {
                                    <$setting as $crate::Setting>::Value::from_file_value(&json_val)
                                })
                                .or_else(|| serde_json::from_str(&value).ok());
                            let Some(value) = value else {
                                return Err(anyhow!(
                                    "Failed to parse loaded value for setting {}: Not loading",
                                    $setting::storage_key()
                                ));
                            };
                            settings_group_load_clone.update(ctx, |settings_group, ctx| {
                                settings_group.$var.load_value(value, explicitly_set, ctx)
                            })
                        },
                        |left, right| {
                            use $crate::_settings_value::SettingsValue as _;
                            let parse =
                                |s: &str| -> anyhow::Result<<$setting as $crate::Setting>::Value> {
                                    let json_val = serde_json::from_str::<serde_json::Value>(s)?;
                                    <$setting as $crate::Setting>::Value::from_file_value(&json_val)
                                        .or_else(|| serde_json::from_str(s).ok())
                                        .ok_or_else(|| {
                                            anyhow!(
                                                "Failed to parse value for {}",
                                                $setting::storage_key()
                                            )
                                        })
                                };
                            let left_setting = $setting::new(Some(parse(left)?));
                            let right_setting = $setting::new(Some(parse(right)?));
                            Ok(left_setting.value() == right_setting.value())
                        },
                        move |ctx| {
                            settings_group_is_syncable_clone
                                .as_ref(ctx)
                                .$var
                                .current_value_is_syncable()
                        },
                    );
                });
            }
        });
    };
}
pub use generate_settings_event_fn;

#[cfg(test)]
#[path = "macros_tests.rs"]
mod macros_tests;
