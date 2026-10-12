#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationDemand {
    pub window_visible: bool,
    pub minimized: bool,
    pub page_active: bool,
    pub component_visible: bool,
    pub intersects_viewport: bool,
}

impl PresentationDemand {
    pub fn needs_subscription(self) -> bool {
        self.window_visible
            && !self.minimized
            && self.page_active
            && self.component_visible
            && self.intersects_viewport
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationRect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl PresentationRect {
    pub fn intersects_viewport(self, viewport: Self, clipping_ancestors: &[Self]) -> bool {
        if !self.valid() {
            return false;
        }
        let mut left = self.left;
        let mut top = self.top;
        let mut right = left + self.width;
        let mut bottom = top + self.height;
        for clip in std::iter::once(&viewport).chain(clipping_ancestors) {
            if !clip.valid() {
                return false;
            }
            left = left.max(clip.left);
            top = top.max(clip.top);
            right = right.min(clip.left + clip.width);
            bottom = bottom.min(clip.top + clip.height);
            if right <= left || bottom <= top {
                return false;
            }
        }
        true
    }

    fn valid(self) -> bool {
        self.left.is_finite()
            && self.top.is_finite()
            && self.width.is_finite()
            && self.width > 0.0
            && self.height.is_finite()
            && self.height > 0.0
            && (self.left + self.width).is_finite()
            && (self.top + self.height).is_finite()
    }
}
use crate::service::{AppHandle, DispatchError, SubscriptionDeliveryError, SubscriptionTicket};
use moiren_signal::{Shape, Subscriber, SubscriptionOptions, Topic};

pub struct VisibleSignal<P: Shape> {
    handle: AppHandle,
    topic: Topic<P>,
    options: SubscriptionOptions,
    ticket: Option<SubscriptionTicket<P>>,
    subscriber: Option<Subscriber<P>>,
}

impl<P: Shape> VisibleSignal<P> {
    pub fn new(handle: AppHandle, topic: Topic<P>, options: SubscriptionOptions) -> Self {
        Self {
            handle,
            topic,
            options,
            ticket: None,
            subscriber: None,
        }
    }

    pub fn update_demand(&mut self, demand: PresentationDemand) -> Result<(), DispatchError> {
        if !demand.needs_subscription() {
            self.clear();
        } else if self.ticket.is_none() && self.subscriber.is_none() {
            self.ticket = Some(
                self.handle
                    .subscribe_topic(self.topic.clone(), self.options)?,
            );
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.ticket = None;
        self.subscriber = None;
    }

    pub fn subscriber(&mut self) -> Result<Option<&mut Subscriber<P>>, SubscriptionDeliveryError> {
        if let Some(ticket) = self.ticket.as_mut() {
            match ticket.try_take() {
                Ok(Some(subscriber)) => {
                    self.ticket = None;
                    self.subscriber = Some(subscriber);
                }
                Ok(None) => {}
                Err(error) => {
                    self.ticket = None;
                    return Err(error);
                }
            }
        }
        Ok(self.subscriber.as_mut())
    }
}
