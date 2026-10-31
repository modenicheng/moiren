//! Test observation hook shared by the window shell and the shell-less pages.
//!
//! The trait lives in a neutral module so pages never reach up into `window`
//! for it: pages stay reusable under other shells.

#[cfg(feature = "gpui-test")]
pub(crate) trait TestObserve {
    type Out;
    fn gpui_test_observe(self) -> Self::Out;
}
#[cfg(feature = "gpui-test")]
impl<E: gpui_kit::base::TestSupportExt> TestObserve for E {
    type Out = gpui_kit::base::ObservedElement<E>;
    fn gpui_test_observe(self) -> Self::Out {
        gpui_kit::base::TestSupportExt::test_support(self)
    }
}
#[cfg(not(feature = "gpui-test"))]
pub(crate) trait TestObserve {
    fn gpui_test_observe(self) -> Self;
}
#[cfg(not(feature = "gpui-test"))]
impl<E> TestObserve for E {
    fn gpui_test_observe(self) -> Self {
        self
    }
}
