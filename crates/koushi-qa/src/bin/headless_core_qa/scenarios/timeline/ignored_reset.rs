use super::*;

pub(super) async fn verify(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    preserved_event_id: &str,
    ignored_user_id: &str,
) -> Result<(), String> {
    for ignored in [true, false] {
        let request_id = conn.next_request_id();
        let command = if ignored {
            AccountCommand::IgnoreUser {
                request_id,
                user_id: ignored_user_id.to_owned(),
            }
        } else {
            AccountCommand::UnignoreUser {
                request_id,
                user_id: ignored_user_id.to_owned(),
            }
        };
        conn.command(CoreCommand::Account(command))
            .await
            .map_err(|_| "ignored reset: command submission failed")?;
        let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
        let mut saw_clear = false;
        let mut items = Vec::new();
        loop {
            let event = deadline
                .recv(conn)
                .await
                .map_err(|_| {
                    if saw_clear {
                        "ignored reset: history did not return after Clear"
                    } else {
                        "ignored reset: no Clear observed"
                    }
                })?
                .map_err(|_| "ignored reset: event stream lagged")?;
            match event {
                CoreEvent::OperationFailed {
                    request_id: failed_id,
                    ..
                } if failed_id == request_id => {
                    return Err("ignored reset: ignore command failed".into());
                }
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                    key: event_key,
                    diffs,
                    ..
                }) if event_key == *key => {
                    for diff in &diffs {
                        if matches!(diff, TimelineDiff::Clear)
                            || matches!(diff, TimelineDiff::Reset { items } if items.iter().all(|item| item.sender.is_none()))
                        {
                            saw_clear = true;
                        }
                        apply_timeline_diff(&mut items, diff);
                    }
                }
                _ => {}
            }
            let state = conn.snapshot();
            if saw_clear
                && state.profile.ignored_user_update.is_idle()
                && state.profile.ignored_user_ids.contains(ignored_user_id) == ignored
                && items.iter().any(|item| {
                    timeline_item_event_id(item) == Some(preserved_event_id) && !item.is_hidden
                })
            {
                break;
            }
        }
    }
    println!("ignored_user_history_recovery=ok");
    Ok(())
}
