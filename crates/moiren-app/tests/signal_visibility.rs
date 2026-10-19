mod common;

use moiren_app::service::{AppRequest, RequestId, ServiceMailbox};
use moiren_app::ui::signals::{PresentationDemand, PresentationRect, VisibleSignal};
use std::time::Duration;

fn visible() -> PresentationDemand {
    PresentationDemand {
        window_visible: true,
        minimized: false,
        page_active: true,
        component_visible: true,
        intersects_viewport: true,
    }
}

fn rect(left: f32, top: f32, width: f32, height: f32) -> PresentationRect {
    PresentationRect {
        left,
        top,
        width,
        height,
    }
}

#[test]
fn every_presentation_predicate_is_required_independently() {
    assert!(visible().needs_subscription());
    let mut cases = [visible(); 5];
    cases[0].window_visible = false;
    cases[1].minimized = true;
    cases[2].page_active = false;
    cases[3].component_visible = false;
    cases[4].intersects_viewport = false;
    for demand in cases {
        assert!(!demand.needs_subscription(), "{demand:?}");
    }
}

#[test]
fn partial_intersection_is_active_and_fully_clipped_regions_are_inactive() {
    let viewport = rect(0.0, 0.0, 100.0, 100.0);
    assert!(rect(80.0, 80.0, 40.0, 40.0).intersects_viewport(viewport, &[]));
    assert!(!rect(100.0, 0.0, 10.0, 20.0).intersects_viewport(viewport, &[]));
    assert!(!rect(0.0, -20.0, 10.0, 20.0).intersects_viewport(viewport, &[]));
    let meter = rect(50.0, 50.0, 20.0, 20.0);
    assert!(!meter.intersects_viewport(viewport, &[rect(0.0, 0.0, 40.0, 100.0)]));
    assert!(!meter.intersects_viewport(viewport, &[rect(0.0, 0.0, 100.0, 40.0)]));
    assert!(meter.intersects_viewport(viewport, &[rect(0.0, 0.0, 60.0, 60.0)]));
}

#[test]
fn empty_invalid_and_disjoint_clipping_regions_have_no_demand() {
    let viewport = rect(0.0, 0.0, 100.0, 100.0);
    assert!(!rect(50.0, 50.0, 0.0, 20.0).intersects_viewport(viewport, &[]));
    assert!(!rect(50.0, 50.0, -2.0, 20.0).intersects_viewport(viewport, &[]));
    assert!(!rect(f32::NAN, 50.0, 20.0, 20.0).intersects_viewport(viewport, &[]));
    assert!(
        !rect(50.0, 50.0, 20.0, 20.0)
            .intersects_viewport(viewport, &[rect(0.0, 0.0, f32::INFINITY, 40.0)])
    );
    assert!(!rect(0.0, 0.0, 100.0, 100.0).intersects_viewport(
        viewport,
        &[rect(0.0, 0.0, 40.0, 100.0), rect(60.0, 0.0, 40.0, 100.0)]
    ));
}

#[test]
fn persistent_visible_ticks_enqueue_only_one_subscription() {
    let (handle, _mailbox) = ServiceMailbox::new(std::thread::current());
    let topic = handle
        .try_snapshot()
        .unwrap()
        .signal_topics
        .tone_output
        .clone();
    let mut visible_signal = VisibleSignal::new(handle.clone(), topic, Default::default());
    visible_signal.update_demand(visible()).unwrap();
    visible_signal.update_demand(visible()).unwrap();
    assert_eq!(
        handle
            .try_request_tracked(AppRequest::RefreshCatalog)
            .unwrap(),
        RequestId(2)
    );
}

#[test]
fn hide_show_between_audio_blocks_advances_real_demand_and_retains_analysis() {
    let runtime = common::service();
    let handle = runtime.handle();
    let topic = common::wait_snapshot(&handle)
        .signal_topics
        .tone_output
        .clone();
    let mut visible_signal = VisibleSignal::new(handle.clone(), topic.clone(), Default::default());
    visible_signal.update_demand(visible()).unwrap();
    common::wait_until(Duration::from_secs(2), || {
        visible_signal.subscriber().unwrap().is_some()
    });
    assert_eq!(topic.business_subscribers(), 1);
    let first = topic.demand_snapshot().generation;
    visible_signal.clear();
    assert_eq!(topic.business_subscribers(), 0);
    visible_signal.update_demand(visible()).unwrap();
    common::wait_until(Duration::from_secs(2), || {
        visible_signal.subscriber().unwrap().is_some()
    });
    assert!(topic.demand_snapshot().generation > first);
    let mut analysis_ticket = handle
        .subscribe_topic(topic.clone(), Default::default())
        .unwrap();
    let analysis = common::wait_for(Duration::from_secs(2), || {
        analysis_ticket.try_take().unwrap()
    });
    drop(visible_signal);
    assert_eq!(topic.business_subscribers(), 1);
    assert!(topic.demand_snapshot().active);
    drop(analysis);
    assert_eq!(topic.business_subscribers(), 0);
    common::finish(runtime);
}

#[test]
fn each_hide_predicate_drops_only_ui_demand_and_keeps_analysis_generation() {
    let runtime = common::service();
    let handle = runtime.handle();
    let topic = common::wait_snapshot(&handle)
        .signal_topics
        .tone_output
        .clone();
    let mut ticket = handle
        .subscribe_topic(topic.clone(), Default::default())
        .unwrap();
    let analysis = common::wait_for(Duration::from_secs(2), || ticket.try_take().unwrap());
    drop(ticket);
    let generation = topic.demand_snapshot().generation;
    let mut ui = VisibleSignal::new(handle.clone(), topic.clone(), Default::default());
    let mut hidden = [visible(); 5];
    hidden[0].window_visible = false;
    hidden[1].minimized = true;
    hidden[2].page_active = false;
    hidden[3].component_visible = false;
    hidden[4].intersects_viewport = false;
    for demand in hidden {
        ui.update_demand(visible()).unwrap();
        common::wait_until(Duration::from_secs(2), || {
            ui.subscriber().unwrap().is_some()
        });
        assert_eq!(topic.business_subscribers(), 2);
        ui.update_demand(demand).unwrap();
        ui.update_demand(demand).unwrap();
        assert!(ui.subscriber().unwrap().is_none());
        assert_eq!(topic.business_subscribers(), 1);
        assert!(topic.demand_snapshot().active);
        assert_eq!(topic.demand_snapshot().generation, generation);
    }
    drop(ui);
    assert_eq!(topic.business_subscribers(), 1);
    drop(analysis);
    assert_eq!(topic.business_subscribers(), 0);
    common::finish(runtime);
}
