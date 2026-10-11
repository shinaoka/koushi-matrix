//! Windows release-check backend.
//!
//! This backend is intentionally notification-only. It reads public GitHub
//! Release metadata and reports a newer version, but download/install remains
//! rejected by the platform-neutral adapter until signed updater artifacts are
//! published and verified.

use serde::Deserialize;
use tauri::{AppHandle, Emitter};

use super::channel_policy::{candidate_version_is_newer, select_newer_candidate};
use super::{
    Backend, Completion, DESKTOP_UPDATE_EVENT_NAME, DesktopUpdateState, PendingUpdate,
    UpdateFuture, Work,
};

const RELEASES_API: &str = "https://api.github.com/repos/shinaoka/koushi-matrix/releases";

pub(super) type Candidate = GitHubRelease;

#[derive(Clone, Debug)]
pub(super) struct GitHubRelease {
    pub(super) version: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

pub(super) struct WindowsBackend {
    app: AppHandle,
    current_version: String,
}

impl WindowsBackend {
    pub(super) fn for_app(app: &AppHandle) -> Option<Self> {
        Some(Self {
            app: app.clone(),
            current_version: app.package_info().version.to_string(),
        })
    }
}

impl Backend<GitHubRelease> for WindowsBackend {
    fn start(&self, work: Work<GitHubRelease>) -> UpdateFuture<'static, Completion<GitHubRelease>> {
        let current_version = self.current_version.clone();
        match work {
            Work::Check(include_prereleases) => Box::pin(async move {
                let result = check_releases(&current_version, include_prereleases).await;
                Completion::Check(result.map(|candidate| {
                    candidate.map(|candidate| PendingUpdate {
                        version: candidate.version.clone(),
                        notification_only: true,
                        update: candidate,
                        bytes: None,
                    })
                }))
            }),
            Work::Download(_) => Box::pin(async { Completion::Download(Err(())) }),
            Work::Install(_) => Box::pin(async { Completion::Install(Err(())) }),
        }
    }

    fn emit(&self, state: DesktopUpdateState) {
        let _ = self.app.emit(DESKTOP_UPDATE_EVENT_NAME, state);
    }

    fn current_version(&self) -> &str {
        &self.current_version
    }

    fn restart(&self) {
        // Windows installation is deliberately disabled until the signed
        // updater artifact contract is complete.
    }
}

async fn check_releases(
    current_version: &str,
    include_prereleases: bool,
) -> Result<Option<GitHubRelease>, ()> {
    let client = reqwest::Client::builder()
        .user_agent("Koushi desktop update checker")
        .build()
        .map_err(|_| ())?;
    let releases = client
        .get(RELEASES_API)
        .query(&[("per_page", "30")])
        .send()
        .await
        .map_err(|_| ())?
        .error_for_status()
        .map_err(|_| ())?
        .json::<Vec<ReleaseResponse>>()
        .await
        .map_err(|_| ())?;

    let mut selected = None;
    for release in releases {
        if release.draft || (!include_prereleases && release.prerelease) {
            continue;
        }
        let Some(version) = release.tag_name.strip_prefix('v') else {
            continue;
        };
        if !is_valid_semver(version) || !candidate_version_is_newer(current_version, version) {
            continue;
        }
        let candidate = GitHubRelease {
            version: version.to_owned(),
        };
        selected = Some(select_newer_candidate(selected, candidate, |candidate| {
            candidate.version.as_str()
        }));
    }
    Ok(selected)
}

fn is_valid_semver(version: &str) -> bool {
    let version = version.split_once('+').map_or(version, |(value, _)| value);
    let (core, prerelease) = version
        .split_once('-')
        .map_or((version, None), |(core, pre)| (core, Some(pre)));
    let core = core.split('.').collect::<Vec<_>>();
    if core.len() != 3
        || core.iter().any(|part| {
            part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || part.parse::<u64>().is_err()
        })
    {
        return false;
    }
    prerelease.is_none_or(|value| {
        !value.is_empty()
            && value.split('.').all(|part| {
                !part.is_empty()
                    && (!part.chars().all(|character| character.is_ascii_digit())
                        || (part.len() == 1 || !part.starts_with('0')))
                    && part
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '-')
            })
    })
}
