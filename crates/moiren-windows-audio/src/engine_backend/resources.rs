use super::{WindowsBackendConfig, selectors};
use moiren_core::graph::NodeId;
use moiren_engine::{
    host::{BackendError, CancellationToken, EngineProject, NodeConfig, ResourceEvent},
    node::{DefaultDeviceRole, EndpointSelector, InputSource, OutputTarget},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
};
use windows::{
    Win32::{
        Media::Audio::{
            DEVICE_STATE_ACTIVE, IMMDeviceEnumerator, MMDeviceEnumerator, eCapture,
            eCommunications, eConsole, eMultimedia, eRender,
        },
        System::Com::{CLSCTX_ALL, CoCreateInstance},
    },
    core::HSTRING,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResourceSelector {
    Endpoint {
        endpoint: EndpointSelector,
        capture: bool,
    },
    Process {
        pid: u32,
        creation: u64,
    },
}

struct Watch {
    selector: ResourceSelector,
    configuration: NodeConfig,
    identity: Option<String>,
    observed: bool,
    retry: bool,
}

#[derive(Default)]
struct State {
    watches: BTreeMap<NodeId, Watch>,
    events: BTreeMap<NodeId, ResourceEvent>,
    generation: u64,
}
impl State {
    fn observe(&mut self, node: NodeId, selector: &ResourceSelector, identity: Option<String>) {
        let Some(watch) = self.watches.get_mut(&node) else {
            return;
        };
        if watch.selector != *selector {
            return;
        }
        let changed = watch.observed && watch.identity != identity;
        let recovered = watch.retry && identity.is_some();
        watch.identity = identity.clone();
        watch.observed = true;
        if recovered {
            watch.retry = false;
        }
        if changed || recovered {
            self.generation = self.generation.saturating_add(1);
            self.events.insert(
                node,
                ResourceEvent {
                    node,
                    configuration: watch.configuration.clone(),
                    generation: self.generation,
                    available: identity.is_some(),
                },
            );
        }
    }
}

pub(super) struct Resources {
    state: Arc<Mutex<State>>,
    cancellation: CancellationToken,
    worker: Option<JoinHandle<()>>,
    capacity: usize,
}

impl Resources {
    pub(super) fn new(config: WindowsBackendConfig) -> Result<Self, BackendError> {
        let state = Arc::new(Mutex::new(State::default()));
        let cancellation = CancellationToken::default();
        let worker_state = state.clone();
        let worker_cancel = cancellation.clone();
        let worker = thread::Builder::new()
            .name("moiren-resource-probe".into())
            .spawn(move || {
                while !worker_cancel.wait_cancelled(config.probe_interval) {
                    let probes: Vec<_> = worker_state
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .watches
                        .iter()
                        .map(|(&node, watch)| (node, watch.selector.clone()))
                        .collect();
                    // One worker probes serially: each resource has at most one
                    // outstanding probe, with no blocking work on the coordinator.
                    for (node, selector) in probes {
                        if worker_cancel.is_cancelled() {
                            break;
                        }
                        let identity = resolve(&selector).ok();
                        let mut state = worker_state
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        state.observe(node, &selector, identity);
                    }
                }
            })
            .map_err(|error| BackendError::Failed(error.to_string()))?;
        Ok(Self {
            state,
            cancellation,
            worker: Some(worker),
            capacity: config.max_resources,
        })
    }

    pub(super) fn track(
        &self,
        project: &EngineProject,
        generation: u64,
    ) -> Result<(), BackendError> {
        let mut selectors = BTreeMap::new();
        for (&node, config) in &project.nodes {
            let selector = match config {
                NodeConfig::Input(input) => match &input.source {
                    InputSource::Physical(endpoint) => Some(ResourceSelector::Endpoint {
                        endpoint: endpoint.clone(),
                        capture: true,
                    }),
                    InputSource::Application { executable, .. } => {
                        let (pid, creation) = selectors::process(executable)?;
                        Some(ResourceSelector::Process { pid, creation })
                    }
                    _ => None,
                },
                NodeConfig::Output(output) => match &output.target {
                    OutputTarget::Physical(endpoint) => Some(ResourceSelector::Endpoint {
                        endpoint: endpoint.clone(),
                        capture: false,
                    }),
                    _ => None,
                },
                _ => None,
            };
            if let Some(selector) = selector {
                selectors.insert(node, (selector, config.clone()));
            }
        }
        if selectors.len() > self.capacity {
            return Err(BackendError::Unsupported(
                "native resource capacity exceeded".into(),
            ));
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.generation = state.generation.max(generation);
        state.watches.retain(|node, _| selectors.contains_key(node));
        state.events.retain(|node, event| {
            selectors
                .get(node)
                .is_some_and(|(_, configuration)| event.configuration == *configuration)
        });
        for (node, (selector, configuration)) in selectors {
            if state.watches.get(&node).is_some_and(|watch| {
                watch.selector == selector && watch.configuration == configuration
            }) {
                continue;
            }
            state.watches.insert(
                node,
                Watch {
                    selector,
                    configuration,
                    identity: None,
                    observed: false,
                    retry: false,
                },
            );
        }
        Ok(())
    }

    pub(super) fn retry(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for watch in state.watches.values_mut() {
            watch.retry = true;
        }
    }

    pub(super) fn resolve(
        &self,
        node: NodeId,
        selector: ResourceSelector,
    ) -> Result<String, BackendError> {
        let resolved = resolve(&selector);
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .observe(node, &selector, resolved.as_ref().ok().cloned());
        resolved
    }

    pub(super) fn poll(&self) -> Option<ResourceEvent> {
        self.state
            .try_lock()
            .ok()?
            .events
            .pop_first()
            .map(|(_, event)| event)
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) fn resolve(selector: &ResourceSelector) -> Result<String, BackendError> {
    match selector {
        ResourceSelector::Process { pid, creation } => {
            let process = crate::owner::Process::open(*pid)
                .map_err(|error| BackendError::ResourceUnavailable(error.to_string()))?;
            if process.identity.creation_time_100ns != *creation
                || process
                    .exited()
                    .map_err(|error| BackendError::Failed(error.to_string()))?
            {
                return Err(BackendError::ResourceUnavailable(
                    "selected process exited or its identity changed".into(),
                ));
            }
            Ok(format!("{pid}:{creation}"))
        }
        ResourceSelector::Endpoint { endpoint, capture } => {
            let _apartment = crate::owner::Apartment::new()
                .map_err(|error| BackendError::Failed(error.to_string()))?;
            let enumerator: IMMDeviceEnumerator =
                unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                    .map_err(|error| BackendError::Failed(error.to_string()))?;
            let flow = if *capture { eCapture } else { eRender };
            let device = match endpoint {
                EndpointSelector::PinnedEndpoint { endpoint_id, .. } => unsafe {
                    enumerator.GetDevice(&HSTRING::from(endpoint_id))
                },
                EndpointSelector::FollowDefault(role) => unsafe {
                    enumerator.GetDefaultAudioEndpoint(
                        flow,
                        match role {
                            DefaultDeviceRole::Console => eConsole,
                            DefaultDeviceRole::Multimedia => eMultimedia,
                            DefaultDeviceRole::Communications => eCommunications,
                        },
                    )
                },
            }
            .map_err(|error| BackendError::ResourceUnavailable(error.to_string()))?;
            let direction: windows::Win32::Media::Audio::IMMEndpoint = device
                .cast()
                .map_err(|error| BackendError::Failed(error.to_string()))?;
            let active = unsafe { device.GetState() }
                .map_err(|error| BackendError::ResourceUnavailable(error.to_string()))?;
            if active != DEVICE_STATE_ACTIVE
                || unsafe { direction.GetDataFlow() }
                    .map_err(|error| BackendError::Failed(error.to_string()))?
                    != flow
            {
                return Err(BackendError::ResourceUnavailable(
                    "selected endpoint is inactive or has the wrong direction".into(),
                ));
            }
            crate::owner::take_string(
                unsafe { device.GetId() }
                    .map_err(|error| BackendError::Failed(error.to_string()))?,
            )
            .map_err(|error| BackendError::Failed(error.to_string()))
        }
    }
}

use windows::core::Interface;

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::{LogicalGraph, NodeKind};

    fn state(selector: ResourceSelector) -> (State, NodeId) {
        let node = LogicalGraph::new().create_node(NodeKind::Sink, 2).unwrap();
        let mut state = State::default();
        state.watches.insert(
            node,
            Watch {
                selector,
                configuration: NodeConfig::Output(super::super::physical_output(
                    EndpointSelector::FollowDefault(DefaultDeviceRole::Multimedia),
                )),
                identity: None,
                observed: false,
                retry: false,
            },
        );
        (state, node)
    }

    #[test]
    fn default_identity_changes_and_availability_events_are_coalesced_and_monotonic() {
        let selector = ResourceSelector::Endpoint {
            endpoint: EndpointSelector::FollowDefault(DefaultDeviceRole::Multimedia),
            capture: false,
        };
        let (mut state, node) = state(selector.clone());
        state.observe(node, &selector, Some("first".into()));
        assert!(state.events.is_empty());
        state.observe(node, &selector, Some("second".into()));
        assert_eq!(state.events[&node].generation, 1);
        state.observe(node, &selector, Some("second".into()));
        assert_eq!(state.generation, 1);
        state.observe(node, &selector, None);
        state.observe(node, &selector, Some("second".into()));
        assert_eq!(state.events.len(), 1);
        assert_eq!(
            state.events[&node],
            ResourceEvent {
                node,
                configuration: state.watches[&node].configuration.clone(),
                generation: 3,
                available: true
            }
        );
    }

    #[test]
    fn recoverable_retry_is_bounded_and_stale_selector_results_are_ignored() {
        let selector = ResourceSelector::Process {
            pid: 1,
            creation: 2,
        };
        let (mut state, node) = state(selector.clone());
        state.watches.get_mut(&node).unwrap().retry = true;
        state.observe(node, &selector, None);
        assert!(state.events.is_empty());
        state.observe(node, &selector, Some("1:2".into()));
        assert_eq!(state.events[&node].generation, 1);
        state.observe(node, &selector, Some("1:2".into()));
        assert_eq!(state.generation, 1);
        state.observe(
            node,
            &ResourceSelector::Process {
                pid: 1,
                creation: 3,
            },
            Some("1:3".into()),
        );
        assert_eq!(state.watches[&node].identity.as_deref(), Some("1:2"));
        assert_eq!(state.generation, 1);
    }
}
