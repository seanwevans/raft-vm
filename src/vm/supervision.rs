use crate::vm::error::VmError;

/// Restart policies supported by supervisor actors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorStrategy {
    OneForOne,
    OneForAll,
    RestForOne,
}

impl SupervisorStrategy {
    pub fn from_usize(value: usize) -> Self {
        match value {
            1 => SupervisorStrategy::OneForAll,
            2 => SupervisorStrategy::RestForOne,
            _ => SupervisorStrategy::OneForOne,
        }
    }
}

/// A child entry tracked by a supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildSpec {
    pub reference: usize,
    pub start_ip: usize,
}

/// Stateful supervision metadata stored on supervisor VMs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorState {
    strategy: SupervisorStrategy,
    children: Vec<ChildSpec>,
}

impl Default for SupervisorState {
    fn default() -> Self {
        Self {
            strategy: SupervisorStrategy::OneForOne,
            children: Vec::new(),
        }
    }
}

impl SupervisorState {
    pub fn strategy(&self) -> SupervisorStrategy {
        self.strategy
    }

    pub fn set_strategy(&mut self, strategy: SupervisorStrategy) {
        self.strategy = strategy;
    }

    pub fn children(&self) -> &[ChildSpec] {
        &self.children
    }

    pub fn ensure_child(&mut self, child: ChildSpec) {
        if let Some(existing) = self
            .children
            .iter_mut()
            .find(|existing| existing.reference == child.reference)
        {
            *existing = child;
        } else {
            self.children.push(child);
        }
    }

    pub fn restart_targets(&mut self, child: ChildSpec) -> Vec<ChildSpec> {
        self.ensure_child(child);
        let child_index = self
            .children
            .iter()
            .position(|existing| existing.reference == child.reference)
            .unwrap_or(0);

        match self.strategy {
            SupervisorStrategy::OneForOne => vec![self.children[child_index]],
            SupervisorStrategy::OneForAll => self.children.clone(),
            SupervisorStrategy::RestForOne => self.children[child_index..].to_vec(),
        }
    }
}

/// Coarse-grained exit reasons that can be sent through actor mailboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    Normal,
    DivisionByZero,
    TypeMismatch,
    Error,
}

impl From<&VmError> for ExitReason {
    fn from(error: &VmError) -> Self {
        match error {
            VmError::DivisionByZero => ExitReason::DivisionByZero,
            VmError::TypeMismatch(_) => ExitReason::TypeMismatch,
            _ => ExitReason::Error,
        }
    }
}

/// Erlang-style exit signal delivered to linked processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitSignal {
    pub from: usize,
    pub reason: ExitReason,
}
