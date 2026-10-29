use super::*;
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

#[derive(Default)]
struct WakeCount(AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn idle_has_no_timer_and_a_question_resumes_quarter_second_checks() {
    let (sender, receiver) = tokio::sync::watch::channel(None);
    let mut schedule = QuestionSchedule::new(receiver);
    let wakes = Arc::new(WakeCount::default());
    let waker = Waker::from(wakes.clone());
    {
        let next = schedule.next_check();
        tokio::pin!(next);
        assert!(matches!(
            next.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        ));
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(matches!(
            next.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        ));
        assert_eq!(
            wakes.0.load(Ordering::SeqCst),
            0,
            "an empty panel must not wake its watcher"
        );
        sender.send_replace(Some("question-one".into()));
        assert_eq!(next.await.as_deref(), Some("question-one"));
    }
    {
        let next = schedule.next_check();
        tokio::pin!(next);
        tokio::time::advance(POLL - Duration::from_millis(1)).await;
        assert!(matches!(
            std::future::poll_fn(|cx| Poll::Ready(next.as_mut().poll(cx))).await,
            Poll::Pending
        ));
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(next.await.as_deref(), Some("question-one"));
    }
    sender.send_replace(None);
    wakes.0.store(0, Ordering::SeqCst);
    {
        let next = schedule.next_check();
        tokio::pin!(next);
        assert!(matches!(
            next.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        ));
        tokio::time::advance(Duration::from_secs(3600)).await;
        assert!(matches!(
            next.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        ));
        assert_eq!(
            wakes.0.load(Ordering::SeqCst),
            0,
            "clearing a question suspends the timer again"
        );
        sender.send_replace(Some("question-two".into()));
        assert_eq!(next.await.as_deref(), Some("question-two"));
    }
}
