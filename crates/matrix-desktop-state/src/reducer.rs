use crate::{action::AppAction, effect::AppEffect, state::AppState};

pub fn reduce(_state: &mut AppState, _action: AppAction) -> Vec<AppEffect> {
    Vec::new()
}
