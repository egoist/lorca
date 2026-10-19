//! The desktop app's own updates, which any Device can start. The app carries this CLI and
//! updates itself through MyGo's updater. While it is connected and can update itself, it tells
//! the CLI its version and how its updates stand (`app.update_status`), which goes up in this
//! Device's `machine` blob as `app`. `device.update_app` names a Runner, and reaches it as the
//! sealed `app.update.install` request; its CLI hands the install to the app
//! (`app.update.install` event). The app installs the version its own signed feed offers, asks
//! `app.relaunch_when_idle`, which answers once no bot is at work on this Runner, and relaunches
//! into the new version, which starts the CLI it carries. From that answer until the app stops
//! this CLI, a turn that would begin waits, so the relaunch cuts none off. A request names no
//! version and no download, as `update.install` does not.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::events::Event;
use crate::model::AppStatus;

/// How long turns wait after the app was told this Runner is idle. The app stops this CLI as it
/// relaunches, well within it; a CLI still running after it was not stopped (one the app found
/// running, or a relaunch that failed) and goes on.
const RELAUNCH_HOLD: Duration = Duration::from_secs(30);

/// What the app on the local websocket says of its updates.
#[derive(Default)]
pub struct AppUpdates {
    /// The local connection the app spoke on, and what it said last.
    status: Mutex<Option<(u64, AppStatus)>>,
    /// Until when a turn that would begin waits: the app was told this Runner is idle.
    held_until: Mutex<Option<Instant>>,
}

impl AppUpdates {
    pub fn status(&self) -> Option<AppStatus> {
        self.status.lock().unwrap().as_ref().map(|(_, status)| status.clone())
    }
}

/// `app.update_status` from the app on local connection `connection`, which `closed` cancels
/// as it closes. A report that comes after that is dropped, as the connection's status was.
pub fn report(app: &Arc<App>, connection: u64, closed: &CancellationToken, params: Value) -> Result<Value, String> {
    let status: AppStatus = serde_json::from_value(params).map_err(|e| format!("bad app status: {e}"))?;
    if status.version.trim().is_empty() {
        return Err("missing version".into());
    }
    {
        let mut current = app.app_updates.status.lock().unwrap();
        if closed.is_cancelled() {
            return Err("the connection closed".into());
        }
        *current = Some((connection, status));
    }
    changed(app);
    Ok(Value::Null)
}

/// Local connection `connection` closed, after its `closed` was cancelled: when the app spoke on
/// it, it quit or lost this CLI, and no Device can ask it for an update any more.
pub fn disconnected(app: &Arc<App>, connection: u64) {
    let removed = {
        let mut status = app.app_updates.status.lock().unwrap();
        let ours = status.as_ref().is_some_and(|(spoke_on, _)| *spoke_on == connection);
        if ours {
            *status = None;
        }
        ours
    };
    if removed {
        changed(app);
    }
}

fn changed(app: &Arc<App>) {
    app.push_machine_blob_if_changed();
    app.emit(app.roster_summary());
}

/// `app.update.install`, from a Device or this one: hands the install to the app, which checks
/// its own feed. Answers at once; the `machine` blob carries how it goes.
pub fn install(app: &Arc<App>) -> Result<Value, String> {
    let Some(status) = app.app_updates.status() else {
        let name = app.this_device_id().and_then(|id| app.device(&id)).map(|d| d.name).unwrap_or_else(|| "this Device".into());
        return Err(format!("Lorca is not open on {name}, or it does not update itself."));
    };
    if status.update.state.is_empty() {
        app.emit(Event::AppUpdateInstall);
    }
    Ok(json!({ "version": status.version, "latest": status.update.latest, "installing": true }))
}

/// `app.relaunch_when_idle`: answers once no bot is at work on this Runner and what it queued
/// for the relay went up, so the app can relaunch into the version it installed. Turns that
/// would begin after it answers wait ([`turn_may_start`]). The wait ends with the connection that
/// asked (`closed`): an app that quit relaunches into nothing, and one that comes back asks again.
pub async fn relaunch_when_idle(app: &Arc<App>, closed: &CancellationToken) -> Result<Value, String> {
    let hold = || *app.app_updates.held_until.lock().unwrap() = Some(Instant::now() + RELAUNCH_HOLD);
    tokio::select! {
        _ = crate::update::until_idle(app, hold) => Ok(Value::Null),
        _ = closed.cancelled() => Err("the app's connection closed".into()),
    }
}

/// Returns once a turn may begin: right away, or once the hold that `app.relaunch_when_idle` set
/// has passed, when this CLI is still running.
pub async fn turn_may_start(app: &App) {
    loop {
        let until = *app.app_updates.held_until.lock().unwrap();
        match until {
            Some(until) if until > Instant::now() => tokio::time::sleep_until(until).await,
            _ => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    struct Scratch(Arc<App>, std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> Scratch {
        let home = std::env::temp_dir().join(format!("lorca-app-update-{}", uuid::Uuid::new_v4()));
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Studio".into())).unwrap();
        Scratch(app, home)
    }

    #[tokio::test]
    async fn the_app_on_its_connection_says_how_its_updates_stand_and_takes_the_install() {
        let scratch = scratch_app();
        let app = &scratch.0;
        assert!(install(app).unwrap_err().starts_with("Lorca is not open on Studio"));
        assert_eq!(app.local_device().unwrap().app, None);

        let open = CancellationToken::new();
        report(app, 7, &open, json!({ "version": "0.1.14", "auto": true, "latest": "0.1.15" })).unwrap();
        let device = app.local_device().unwrap();
        assert_eq!(serde_json::to_value(&device.app).unwrap(), json!({ "version": "0.1.14", "auto": true, "latest": "0.1.15" }));
        assert!(report(app, 7, &open, json!({ "auto": true })).is_err(), "no version");

        let mut events = app.events.subscribe();
        assert_eq!(install(app).unwrap(), json!({ "version": "0.1.14", "latest": "0.1.15", "installing": true }));
        assert!(matches!(events.try_recv(), Ok(Event::AppUpdateInstall)));
        // While it installs, another request is answered without asking the app again.
        report(app, 7, &open, json!({ "version": "0.1.14", "auto": true, "latest": "0.1.15", "state": "installing" })).unwrap();
        install(app).unwrap();
        assert!(std::iter::from_fn(|| events.try_recv().ok()).all(|event| !matches!(event, Event::AppUpdateInstall)));

        // Another local client closing leaves it; the app's connection closing takes it away.
        disconnected(app, 8);
        assert!(app.local_device().unwrap().app.is_some());
        disconnected(app, 7);
        assert_eq!(app.local_device().unwrap().app, None);

        // A report that lands after its connection closed does not bring the status back.
        let closed = CancellationToken::new();
        closed.cancel();
        assert!(report(app, 9, &closed, json!({ "version": "0.1.14", "auto": true })).is_err());
        assert_eq!(app.local_device().unwrap().app, None);
    }

    #[tokio::test(start_paused = true)]
    async fn once_the_app_is_told_this_runner_is_idle_new_turns_wait_for_the_relaunch() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let open = CancellationToken::new();
        // Idle at once: the hold starts with the answer.
        relaunch_when_idle(app, &open).await.unwrap();
        let held = Instant::now();
        turn_may_start(app).await;
        assert!(held.elapsed() >= RELAUNCH_HOLD, "a turn that would begin waits for the app to stop this CLI");

        // Without a hold, a turn begins at once.
        let at = Instant::now();
        turn_may_start(app).await;
        assert_eq!(at.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_for_idle_ends_with_the_connection_that_asked() {
        let scratch = scratch_app();
        let app = &scratch.0;
        app.running_jobs.lock().unwrap().insert(
            "turn".into(),
            crate::app::RunningJob {
                chat_id: "chat".into(),
                bot_id: "bot".into(),
                routine_id: None,
                runner_id: None,
                cancel: CancellationToken::new(),
                activity: None,
            },
        );
        let closed = CancellationToken::new();
        let waiting = tokio::spawn({
            let (app, closed) = (app.clone(), closed.clone());
            async move { relaunch_when_idle(&app, &closed).await }
        });
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert!(!waiting.is_finished(), "a turn is in flight");
        closed.cancel();
        assert!(waiting.await.unwrap().is_err());
        assert!(app.app_updates.held_until.lock().unwrap().is_none(), "no relaunch, no hold");
    }
}
