use koushi_core::account_runtime_manager::AccountTabId;
use tauri::{AppHandle, State};

use crate::{
    CoreRuntimeState, account_tabs_snapshot, allow_account_media_cache_dirs,
    emit_account_tabs_changed,
};

#[tauri::command]
pub async fn list_account_tabs(
    state: State<'_, CoreRuntimeState>,
) -> Result<crate::AccountTabsSnapshot, String> {
    Ok(account_tabs_snapshot(&state.runtime))
}

#[tauri::command]
pub async fn select_account_tab(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
    tab_id: String,
) -> Result<crate::AccountTabsSnapshot, String> {
    let tab_id = AccountTabId::from_string(tab_id);
    let previous_tab = state.runtime.selected_tab_id();
    let changed = previous_tab != tab_id;
    if !state
        .runtime
        .select_tab(&tab_id)
        .await
        .map_err(|_| "could not save selected account".to_owned())?
    {
        return Err("account tab does not exist".to_owned());
    }
    if changed {
        state.close_reader_subscriptions().await;
        super::native_attention::transfer_native_window_focus(
            &state.runtime,
            &state.native_window_focus_generation,
            &previous_tab,
            &tab_id,
            state
                .native_window_focused
                .load(std::sync::atomic::Ordering::Relaxed),
        )
        .await;
        state.restart_selected_forwarder(app.clone());
    }
    emit_account_tabs_changed(&app, &state.runtime);
    allow_account_media_cache_dirs(&app, &state.runtime);
    Ok(account_tabs_snapshot(&state.runtime))
}

#[tauri::command]
pub async fn add_account_tab(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<crate::AccountTabsSnapshot, String> {
    let previous_tab = state.runtime.selected_tab_id();
    let selected_tab = state
        .runtime
        .add_account_tab()
        .await
        .map_err(|_| "could not add account tab".to_owned())?;
    state.close_reader_subscriptions().await;
    super::native_attention::transfer_native_window_focus(
        &state.runtime,
        &state.native_window_focus_generation,
        &previous_tab,
        &selected_tab,
        state
            .native_window_focused
            .load(std::sync::atomic::Ordering::Relaxed),
    )
    .await;
    state.restart_selected_forwarder(app.clone());
    state.restart_account_tab_watchers(app.clone()).await;
    emit_account_tabs_changed(&app, &state.runtime);
    Ok(account_tabs_snapshot(&state.runtime))
}

#[tauri::command]
pub async fn remove_signed_out_account_tab(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
    tab_id: String,
) -> Result<crate::AccountTabsSnapshot, String> {
    let tab_id = AccountTabId::from_string(tab_id);
    let selected = state.runtime.selected_tab_id() == tab_id;
    if selected {
        state.close_reader_subscriptions().await;
        state.stop_selected_forwarder().await;
    }
    state.connection.remove_cached_connection(&tab_id).await;
    state.stop_account_tab_watchers().await;
    let removed = match state.runtime.remove_signed_out_tab(&tab_id).await {
        Ok(removed) => removed,
        Err(_) => {
            if selected {
                let selected_tab = state.runtime.selected_tab_id();
                super::native_attention::transfer_native_window_focus(
                    &state.runtime,
                    &state.native_window_focus_generation,
                    &tab_id,
                    &selected_tab,
                    state
                        .native_window_focused
                        .load(std::sync::atomic::Ordering::Relaxed),
                )
                .await;
                state.restart_selected_forwarder(app.clone());
            }
            state.restart_account_tab_watchers(app.clone()).await;
            emit_account_tabs_changed(&app, &state.runtime);
            allow_account_media_cache_dirs(&app, &state.runtime);
            return Err("could not remove account tab".to_owned());
        }
    };
    if !removed {
        if selected {
            state.restart_selected_forwarder(app.clone());
        }
        state.restart_account_tab_watchers(app.clone()).await;
        return Err("account must be signed out before its tab can be removed".to_owned());
    }
    if selected {
        let selected_tab = state.runtime.selected_tab_id();
        super::native_attention::observe_native_window_focus_for_tab(
            &state.runtime,
            &state.native_window_focus_generation,
            &selected_tab,
            state
                .native_window_focused
                .load(std::sync::atomic::Ordering::Relaxed),
        )
        .await;
        state.restart_selected_forwarder(app.clone());
    }
    state.restart_account_tab_watchers(app.clone()).await;
    emit_account_tabs_changed(&app, &state.runtime);
    allow_account_media_cache_dirs(&app, &state.runtime);
    Ok(account_tabs_snapshot(&state.runtime))
}

#[tauri::command]
pub async fn cancel_add_account_tab(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
    tab_id: String,
) -> Result<crate::AccountTabsSnapshot, String> {
    let tab_id = AccountTabId::from_string(tab_id);
    let selected = state.runtime.selected_tab_id() == tab_id;
    if selected {
        state.close_reader_subscriptions().await;
        state.stop_selected_forwarder().await;
    }
    // Drop the adapter's connection first; the child runtime joins only once
    // every connection to it is gone. An uncancelled tab reconnects lazily.
    state.connection.remove_cached_connection(&tab_id).await;
    state.stop_account_tab_watchers().await;
    let cancelled = state.runtime.cancel_add_account_tab(&tab_id).await;
    let selected_tab = state.runtime.selected_tab_id();
    if selected {
        super::native_attention::transfer_native_window_focus(
            &state.runtime,
            &state.native_window_focus_generation,
            &tab_id,
            &selected_tab,
            state
                .native_window_focused
                .load(std::sync::atomic::Ordering::Relaxed),
        )
        .await;
        state.restart_selected_forwarder(app.clone());
    }
    state.restart_account_tab_watchers(app.clone()).await;
    emit_account_tabs_changed(&app, &state.runtime);
    allow_account_media_cache_dirs(&app, &state.runtime);
    match cancelled {
        Ok(true) => Ok(account_tabs_snapshot(&state.runtime)),
        Ok(false) => Err("account tab cannot be cancelled".to_owned()),
        Err(_) => Err("could not cancel account tab".to_owned()),
    }
}
