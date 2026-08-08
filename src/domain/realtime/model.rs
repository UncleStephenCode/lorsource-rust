#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnRealtimeDelivery {
    Comment(i32),
    EventsRefresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StTopicSubscriptionRequest {
    pub iTopicId: i32,
    pub iLastSeenCommentId: i32,
}
