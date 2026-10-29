//! Recording warnings must also appear while Orbit is the foreground app.
use objc2::{ClassType, define_class, msg_send, rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSObject, NSObjectProtocol};
use objc2_user_notifications::{
    UNNotification, UNNotificationPresentationOptions, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

define_class!(
    // NSObject has no subclassing requirements; this delegate has no state.
    #[unsafe(super = NSObject)]
    struct NotificationPresentation;
    unsafe impl NSObjectProtocol for NotificationPresentation {}
    unsafe impl UNUserNotificationCenterDelegate for NotificationPresentation {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }
    }
);

thread_local! {
    // UNUserNotificationCenter's delegate is weak. Setup retains it for the
    // main thread's lifetime; callbacks themselves can arrive on any thread.
    static PRESENTATION: Retained<NotificationPresentation> = unsafe {
        msg_send![NotificationPresentation::class(), new]
    };
}
pub fn install() {
    PRESENTATION.with(|delegate| {
        UNUserNotificationCenter::currentNotificationCenter()
            .setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
    });
}
