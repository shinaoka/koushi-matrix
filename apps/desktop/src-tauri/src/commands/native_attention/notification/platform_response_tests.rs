use super::*;

#[test]
fn only_the_default_notification_response_navigates() {
    use notify_rust::{CloseReason, NotificationResponse};

    assert!(notification_response_activates(
        &NotificationResponse::Default
    ));
    assert!(!notification_response_activates(
        &NotificationResponse::Action("other".to_owned())
    ));
    assert!(!notification_response_activates(
        &NotificationResponse::Closed(CloseReason::Dismissed)
    ));
}

/// #1907: the Linux banner is shown as Koushi and stays clickable. The hints are
/// asserted here because `notify-rust` keeps them on the notification; dropping
/// one of them would otherwise still compile and pass every other test.
#[cfg(target_os = "linux")]
#[test]
fn a_linux_banner_is_koushi_and_carries_the_default_click_action() {
    let payload = banner_payload("Alice: hello", None);
    let notification = linux_banner(&linux_banner_spec(&payload, "Koushi"));
    assert_eq!(notification.summary, "Message in Room");
    assert_eq!(notification.body, "Alice: hello");
    assert_eq!(notification.appname, "Koushi");
    assert!(
        notification
            .hints
            .contains(&notify_rust::Hint::DesktopEntry("Koushi".to_owned())),
        "Plasma reads the app name, icon, and receive time from the desktop entry"
    );
    assert!(
        notification
            .hints
            .contains(&notify_rust::Hint::Category("im.received".to_owned()))
    );
    assert_eq!(notification.actions, vec!["default", "Open"]);
}

#[cfg(windows)]
#[test]
fn windows_installed_banner_uses_its_own_app_identity() {
    assert_eq!(
        notification_app_id(false, "chat.koushi.desktop"),
        Some("chat.koushi.desktop")
    );
    assert_eq!(notification_app_id(true, "chat.koushi.desktop"), None);
}

#[cfg(any(target_os = "linux", test))]
fn banner_payload(body: &str, avatar: Option<&str>) -> NativeNotificationPayload {
    NativeNotificationPayload {
        title: "Message in Room".to_owned(),
        body: body.to_owned(),
        sender_avatar: avatar.map(str::to_owned),
        target: NativeNotificationTarget {
            room_id: "!room:example.invalid".to_owned(),
            event_id: None,
            thread_root_event_id: None,
        },
    }
}

/// #1907: Plasma shows the app name and icon from the installed desktop entry,
/// and the banner keeps the Rust-owned title and text untouched.
#[test]
fn a_linux_banner_names_koushi_and_keeps_the_rust_owned_text() {
    let spec = linux_banner_spec(&banner_payload("Alice: hello", None), "Koushi");
    assert_eq!(spec.summary, "Message in Room");
    assert_eq!(spec.body, "Alice: hello");
    assert_eq!(spec.app_name, "Koushi");
    assert_eq!(spec.desktop_entry, "Koushi");
    assert_eq!(spec.category, "im.received");
    assert_eq!(spec.avatar, None);
}

/// A message is text: Plasma renders markup in the body, so `&`, `<`, and `>`
/// must arrive escaped.
#[test]
fn a_linux_banner_body_is_escaped_for_markup() {
    let spec = linux_banner_spec(&banner_payload("a & b <b>hi</b>", None), "Koushi");
    assert_eq!(spec.body, "a &amp; b &lt;b&gt;hi&lt;/b&gt;");
}

/// The avatar reference is the payload's cache reference; the banner resolves
/// it against the in-memory thumbnail cache, so the OS never sees a URL.
#[test]
fn a_linux_banner_carries_the_payload_avatar_reference() {
    let spec = linux_banner_spec(
        &banner_payload("Alice: hello", Some("avatar/0123456789abcdef")),
        "Koushi",
    );
    assert_eq!(spec.avatar.as_deref(), Some("avatar/0123456789abcdef"));
}

/// An avatar image exists only when that thumbnail is in this session's cache.
#[test]
fn only_a_cached_avatar_thumbnail_becomes_banner_image_data() {
    use koushi_core::renderable_thumbnail::{RenderableThumbnailKind, store_renderable_thumbnail};

    assert_eq!(avatar_rgba(None), None);
    assert_eq!(avatar_rgba(Some("avatar/0000000000000000")), None);

    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2,
        2,
        image::Rgba([1, 2, 3, 255]),
    ));
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode fixture png");
    let koushi_state::AvatarThumbnailState::Ready { source_ref, .. } = store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        "mxc://notification.example.invalid/avatar",
        bytes,
    )
    .expect("cached fixture avatar") else {
        panic!("a stored thumbnail is Ready");
    };
    let (width, height, pixels) = avatar_rgba(Some(&source_ref)).expect("cached avatar image");
    assert_eq!((width, height), (2, 2));
    assert_eq!(pixels, [1, 2, 3, 255].repeat(4));
}
