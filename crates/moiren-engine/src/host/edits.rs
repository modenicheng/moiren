//! Typed editor results are staged with the candidate and published only after
//! the host accepts it. Type erasure and result ownership stay off the RT path.
use super::*;

pub(crate) type AcceptedEdit = Box<dyn FnOnce() + Send>;
pub(crate) type Edit =
    Box<dyn FnOnce(&mut EngineProject) -> Result<AcceptedEdit, ProjectError> + Send>;

impl EngineControl {
    /// Compatibility entry point for edits that do not return object identities.
    pub fn edit(
        &self,
        edit: impl FnOnce(&mut EngineProject) -> Result<(), ProjectError> + Send + 'static,
    ) -> Result<EditReceipt, HostError> {
        self.edit_with_result(edit)
    }

    /// Submit an atomic project edit and retain its return value on acceptance.
    /// A result proves project acceptance, not audio activation. Accepted results
    /// remain available if preparation is superseded or the host later closes.
    pub fn edit_with_result<T: Send + 'static>(
        &self,
        edit: impl FnOnce(&mut EngineProject) -> Result<T, ProjectError> + Send + 'static,
    ) -> Result<EditReceipt<T>, HostError> {
        self.submit_edit(None, edit)
    }

    /// Check the host and graph revision on the coordinator before running the
    /// callback. Pure parameter changes do not invalidate this topology basis;
    /// callbacks needing finer conditions must check them in the candidate.
    pub fn edit_if_current<T: Send + 'static>(
        &self,
        expected: EditBasis,
        edit: impl FnOnce(&mut EngineProject) -> Result<T, ProjectError> + Send + 'static,
    ) -> Result<EditReceipt<T>, HostError> {
        self.submit_edit(Some(expected), edit)
    }

    fn submit_edit<T: Send + 'static>(
        &self,
        expected: Option<EditBasis>,
        edit: impl FnOnce(&mut EngineProject) -> Result<T, ProjectError> + Send + 'static,
    ) -> Result<EditReceipt<T>, HostError> {
        if self.shared.closing.load(Ordering::Acquire) {
            return Err(HostError::Closed);
        }
        let receipt = EditReceipt {
            cell: ReceiptCell::reserve(
                EditOutcome::Queued,
                &self.shared.permits,
                self.receipt_capacity,
            )?,
            result: Arc::new(Mutex::new(None)),
        };
        let result = Arc::clone(&receipt.result);
        let edit: Edit = Box::new(move |project| {
            let value = edit(project)?;
            Ok(Box::new(move || {
                *result.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(value);
            }))
        });
        self.send(Command::Edit {
            expected,
            edit,
            receipt: Arc::clone(&receipt.cell),
        })?;
        Ok(receipt)
    }
}
