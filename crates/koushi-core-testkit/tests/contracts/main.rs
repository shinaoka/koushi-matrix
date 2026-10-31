#![recursion_limit = "512"]

mod account_notification_ownership;
mod command_redaction;
mod composer_draft_lifecycle;
mod composer_draft_wire;
mod dm_space_ids;
mod event_cache_lifecycle;
mod event_redaction;
mod issue_1105_authentication_audit;
mod issue_1105_lifecycle_audit;
mod local_store_migration;
mod login_store_lifecycle;
mod media_staging;
mod media_staging_b2;
mod oauth_authorization_code;
mod pending_login_journal;
mod request_outcome;
mod request_outcome_a2a;
mod request_outcome_a2b;
mod request_outcome_a2c;
mod room_address;
mod room_subscription_residency;
mod send_queue_fast;
mod thread_bundled_reply_timestamp;
