use std::{
    sync::{
        Arc,
        mpsc::{Receiver, SyncSender},
    },
    thread,
};

use moiren_core::protocol::ControlReply;

use super::{BackendError, BackendOwner, Shared, preparation::Ready};
use crate::{
    control::ControlPort,
    runtime::{PlanControlPort, PreparedPlan, RetiredPlan},
    sample::ProcessingSample,
};

pub(crate) enum Cleanup<S: ProcessingSample> {
    Owner {
        owner: Box<dyn BackendOwner<S>>,
        plans: PlanControlPort<S>,
        controls: Vec<ControlPort>,
        signals: moiren_signal::HostControl,
    },
    Retired(RetiredPlan<S>, ControlPort),
    Discard(Ready<S>),
}
pub(crate) struct Reclaimed {
    pub replies: Vec<ControlReply>,
    pub error: Option<BackendError>,
}

pub(crate) fn spawn<S: ProcessingSample>(
    shared: Arc<Shared>,
    jobs: Receiver<Cleanup<S>>,
    completed: SyncSender<Reclaimed>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("moiren-host-cleanup".into())
        .spawn(move || {
            while let Ok(job) = jobs.recv() {
                let result = reclaim(job);
                if completed.send(result).is_err() {
                    break;
                }
                shared.notify();
            }
        })
}

fn drain(control: &mut ControlPort, replies: &mut Vec<ControlReply>) {
    while let Some(reply) = control.poll_applied() {
        replies.push(reply);
    }
}
fn retire<S: ProcessingSample>(
    plan: &mut RetiredPlan<S>,
    control: &mut ControlPort,
    replies: &mut Vec<ControlReply>,
) {
    loop {
        drain(control, replies);
        if plan.reject_pending() == 0 {
            drain(control, replies);
            break;
        }
    }
}
fn candidate<S: ProcessingSample>(
    plan: &mut PreparedPlan<S>,
    control: &mut ControlPort,
    replies: &mut Vec<ControlReply>,
) {
    loop {
        drain(control, replies);
        if plan.retire_controls() == 0 {
            drain(control, replies);
            break;
        }
    }
}
fn reclaim<S: ProcessingSample>(job: Cleanup<S>) -> Reclaimed {
    let mut replies = Vec::new();
    let error = match job {
        Cleanup::Owner {
            owner,
            mut plans,
            mut controls,
            signals,
        } => {
            owner.cancel();
            let returned = owner.stop_and_join();
            let mut engine = returned.engine;
            while let Some(mut plan) = engine.take_pending_plan() {
                if let Some(control) = controls
                    .iter_mut()
                    .find(|control| control.identity().0 == plan.revision())
                {
                    candidate(&mut plan, control, &mut replies);
                }
            }
            while let Some(mut plan) = plans.poll_retired() {
                if let Some(control) = controls
                    .iter_mut()
                    .find(|control| control.identity().0 == plan.revision())
                {
                    retire(&mut plan, control, &mut replies);
                }
            }
            if let Some(control) = controls
                .iter_mut()
                .find(|control| control.identity().0 == engine.revision())
            {
                loop {
                    drain(control, &mut replies);
                    if engine.retire_controls() == 0 {
                        drain(control, &mut replies);
                        break;
                    }
                }
            }
            for control in &mut controls {
                drain(control, &mut replies);
            }
            drop(engine);
            drop(plans);
            drop(controls);
            drop(signals);
            returned.error
        }
        Cleanup::Retired(mut plan, mut control) => {
            retire(&mut plan, &mut control, &mut replies);
            None
        }
        Cleanup::Discard(Ready::Initial(initial)) => {
            return reclaim(Cleanup::Owner {
                owner: initial.owner,
                plans: initial.plans,
                controls: vec![initial.control],
                signals: initial.signals,
            });
        }
        Cleanup::Discard(Ready::Update(mut update)) => {
            candidate(&mut update.plan, &mut update.control, &mut replies);
            None
        }
    };
    Reclaimed { replies, error }
}
