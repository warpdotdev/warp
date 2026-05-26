use url::Url;

use crate::ChannelState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubAuthRedirectTarget {
    SettingsEnvironments,
    FocusCloudMode,
}

impl GithubAuthRedirectTarget {
    fn next_path(self) -> &'static str {
        match self {
            Self::SettingsEnvironments => "settings/environments",
            Self::FocusCloudMode => "action/focus_cloud_mode",
        }
    }
}

/// Indicates where the GitHub authorization flow was initiated from.
/// This affects the redirect URL used after auth completes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthSource {
    /// Auth initiated from the settings page (default behavior: redirect to settings)
    #[default]
    Settings,
    /// Auth initiated from cloud agent setup (skip redirect, just refresh in place)
    CloudSetup,
}

#[derive(Clone, Copy, Debug)]
enum OAuthNextPlatform {
    Native,
    Web,
}

pub fn auth_url_with_next(
    base_auth_url: &str,
    target: GithubAuthRedirectTarget,
    auth_source: AuthSource,
) -> String {
    let scheme = oauth_next_scheme();
    build_auth_url_with_next(base_auth_url, target, &scheme, auth_source)
}
pub fn settings_environments_auth_url_with_next(base_auth_url: &str) -> String {
    auth_url_with_next(
        base_auth_url,
        GithubAuthRedirectTarget::SettingsEnvironments,
        AuthSource::Settings,
    )
}

pub fn cloud_setup_auth_url_with_next(base_auth_url: &str) -> String {
    auth_url_with_next(
        base_auth_url,
        GithubAuthRedirectTarget::FocusCloudMode,
        AuthSource::CloudSetup,
    )
}

pub(crate) fn build_auth_url_with_next(
    base_auth_url: &str,
    target: GithubAuthRedirectTarget,
    scheme: &str,
    auth_source: AuthSource,
) -> String {
    let Ok(mut url) = Url::parse(base_auth_url) else {
        return base_auth_url.to_string();
    };

    let scheme_for_next = std::env::var("WARP_OAUTH_NEXT_SCHEME")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            url.query_pairs()
                .find(|(key, _)| key == "scheme")
                .map(|(_, value)| value.into_owned())
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| scheme.to_string());

    let platform = if cfg!(target_family = "wasm") {
        OAuthNextPlatform::Web
    } else {
        OAuthNextPlatform::Native
    };

    let next_url = build_next_url(target, &scheme_for_next, auth_source, platform)
        .unwrap_or_else(|| format!("{scheme_for_next}://{}", target.next_path()));

    let existing_pairs = url
        .query_pairs()
        .filter(|(key, _)| key != "next")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();

    {
        let mut query_pairs = url.query_pairs_mut();
        query_pairs.clear();
        for (key, value) in existing_pairs {
            query_pairs.append_pair(&key, &value);
        }
        query_pairs.append_pair("next", &next_url);
    }

    url.to_string()
}

fn build_next_url(
    target: GithubAuthRedirectTarget,
    scheme_for_next: &str,
    auth_source: AuthSource,
    platform: OAuthNextPlatform,
) -> Option<String> {
    match platform {
        OAuthNextPlatform::Native => {
            let base = format!("{scheme_for_next}://{}", target.next_path());
            let mut url = Url::parse(&base).ok()?;

            if matches!(auth_source, AuthSource::CloudSetup) {
                url.query_pairs_mut()
                    .append_pair("source", crate::uri::CLOUD_SETUP_SOURCE);
            }

            Some(url.to_string())
        }
        OAuthNextPlatform::Web => {
            let mut url = Url::parse(&ChannelState::server_root_url()).ok()?;
            url.set_query(None);

            match target {
                GithubAuthRedirectTarget::SettingsEnvironments => {
                    url.set_path("/settings/environments");
                    {
                        let mut pairs = url.query_pairs_mut();
                        pairs.append_pair("oauth", "github");
                        if matches!(auth_source, AuthSource::CloudSetup) {
                            pairs.append_pair("source", crate::uri::CLOUD_SETUP_SOURCE);
                        }
                    }
                }
                GithubAuthRedirectTarget::FocusCloudMode => {
                    url.set_path("/action/focus_cloud_mode");
                }
            }

            Some(url.to_string())
        }
    }
}

fn oauth_next_scheme() -> String {
    if let Ok(override_value) = std::env::var("WARP_OAUTH_NEXT_SCHEME") {
        if !override_value.is_empty() {
            return override_value;
        }
    }
    ChannelState::url_scheme().to_string()
}
