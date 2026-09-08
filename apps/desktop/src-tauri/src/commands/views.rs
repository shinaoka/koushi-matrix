use super::*;

#[derive(serde::Serialize)]
pub(crate) struct ReceiptReaderResourceContent {
    pub(crate) bytes: Vec<u8>,
    pub(crate) mime_type: Option<String>,
}

#[tauri::command]
pub async fn subscribe_receipt_reader(
    source: koushi_protocol::view::ReceiptSourceRef,
    start: u64,
    limit: koushi_protocol::view::ReaderWindowLimit,
    state: State<'_, CoreRuntimeState>,
) -> Result<koushi_protocol::view::ViewScopeId, String> {
    let connection = state.connection.lock().await;
    let subscription = connection
        .subscribe_reader(source, start, limit)
        .map_err(|error| format!("reader subscribe failed: {error:?}"))?;
    let scope = subscription.id();
    let entry = crate::ReaderSubscriptionEntry {
        close: subscription.close_handle(),
        subscription: std::sync::Arc::new(tokio::sync::Mutex::new(subscription)),
    };
    state.reader_subscriptions.lock().await.insert(scope, entry);
    Ok(scope)
}

#[tauri::command]
pub async fn receive_receipt_reader(
    scope: koushi_protocol::view::ViewScopeId,
    state: State<'_, CoreRuntimeState>,
) -> Result<Option<koushi_protocol::view::ViewDelivery>, String> {
    let entry = state
        .reader_subscriptions
        .lock()
        .await
        .get(&scope)
        .cloned()
        .ok_or_else(|| "reader scope is not owned by this window".to_owned())?;
    Ok(entry.subscription.lock().await.next_delivery().await)
}

#[tauri::command]
pub async fn read_receipt_reader_resource(
    scope: koushi_protocol::view::ViewScopeId,
    revision: koushi_protocol::view::ViewRevision,
    source_ref: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<Option<ReceiptReaderResourceContent>, String> {
    let entry = state
        .reader_subscriptions
        .lock()
        .await
        .get(&scope)
        .cloned()
        .ok_or_else(|| "reader scope is not owned by this window".to_owned())?;
    entry
        .subscription
        .lock()
        .await
        .resource_content(revision, &source_ref)
        .map(|content| {
            content.map(|content| ReceiptReaderResourceContent {
                bytes: content.bytes,
                mime_type: content.mime_type,
            })
        })
        .map_err(|error| format!("reader resource read failed: {error:?}"))
}

#[tauri::command]
pub async fn update_receipt_reader_window(
    scope: koushi_protocol::view::ViewScopeId,
    request: koushi_protocol::view::ReaderWindowRequest,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let entry = state
        .reader_subscriptions
        .lock()
        .await
        .get(&scope)
        .cloned()
        .ok_or_else(|| "reader scope is not owned by this window".to_owned())?;
    entry
        .subscription
        .lock()
        .await
        .update_window(request)
        .map_err(|error| format!("reader window update failed: {error:?}"))
}

#[tauri::command]
pub async fn ack_receipt_reader(
    scope: koushi_protocol::view::ViewScopeId,
    revision: koushi_protocol::view::ViewRevision,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let entry = state
        .reader_subscriptions
        .lock()
        .await
        .get(&scope)
        .cloned()
        .ok_or_else(|| "reader scope is not owned by this window".to_owned())?;
    entry
        .subscription
        .lock()
        .await
        .ack_model(revision)
        .map_err(|error| format!("reader ACK failed: {error:?}"))
}

#[tauri::command]
pub async fn close_receipt_reader(
    scope: koushi_protocol::view::ViewScopeId,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    if let Some(entry) = state.reader_subscriptions.lock().await.remove(&scope) {
        entry.close.close();
    }
    Ok(())
}

#[tauri::command]
pub async fn open_files_view(
    scope: FilesViewScope,
    filter: AttachmentFilter,
    sort: AttachmentSort,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_open_files_view_command(request_id, scope, filter, sort),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn close_files_view(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_close_files_view_command(request_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn open_threads_list(
    scope: koushi_state::ThreadsListScope,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_open_threads_list_command(request_id, scope),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn close_threads_list(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_close_threads_list_command(request_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn paginate_threads_list(
    scope: koushi_state::ThreadsListScope,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_paginate_threads_list_command(request_id, scope),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn open_thread(
    room_id: String,
    root_event_id: String,
    intent: ThreadOpenIntent,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    // Thread open/close is Rust-owned product state: drive the reducer's
    // ThreadPaneState through a first-class core command instead of discarding
    // the inputs in a snapshot-only shim.
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_open_thread_command(request_id, room_id, root_event_id, intent),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn close_thread(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::App(AppCommand::CloseThread { request_id }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

pub(super) fn build_open_files_view_command(
    request_id: koushi_protocol::RequestId,
    scope: FilesViewScope,
    filter: AttachmentFilter,
    sort: AttachmentSort,
) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenFilesView {
        request_id,
        scope,
        filter,
        sort,
    })
}

pub(super) fn build_close_files_view_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseFilesView { request_id })
}

pub(super) fn build_open_thread_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    root_event_id: String,
    intent: ThreadOpenIntent,
) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenThread {
        request_id,
        room_id,
        root_event_id,
        intent,
    })
}

pub(super) fn build_open_threads_list_command(
    request_id: koushi_protocol::RequestId,
    scope: ThreadsListScope,
) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenThreadsList { request_id, scope })
}

pub(super) fn build_close_threads_list_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseThreadsList { request_id })
}

pub(super) fn build_paginate_threads_list_command(
    request_id: koushi_protocol::RequestId,
    scope: ThreadsListScope,
) -> CoreCommand {
    CoreCommand::App(AppCommand::PaginateThreadsList { request_id, scope })
}
