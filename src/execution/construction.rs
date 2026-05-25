use crate::error::RunError;
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::program::ReturnOutput;

use super::attempt::{RuleAttemptStableReason, RuleMiss};
use super::engine::{AttemptSession, CoreAppliedRule, CoreRuleAttempt, CoreStep, Session};
use super::session::{
    BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRuleAttemptSession, OwnedRunSession,
};
use super::transition::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptReturnedRun,
    BorrowedRuleAttemptStableRun, BorrowedRuleAttemptTransition, BorrowedStableRun,
    BorrowedStepTransition, OwnedAppliedStep, OwnedFailedRun, OwnedMissedRuleAttempt,
    OwnedReturnedRun, OwnedRuleAttemptAppliedStep, OwnedRuleAttemptFailedRun,
    OwnedRuleAttemptReturnedRun, OwnedRuleAttemptStableRun, OwnedRuleAttemptTransition,
    OwnedStableRun, OwnedStepTransition,
};
use super::witness::OwnedRuleWitness;

/// Shared transition construction for ordinary stepwise sessions.
pub(super) trait StepwiseRunSession: Sized {
    /// Public transition produced by this session.
    type Transition;

    /// Rule witness carried by public applied and returned transitions.
    type RuleWitness;

    /// Advances the private runtime session with the right witness boundary.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if stepping through the private runtime session fails.
    fn session_step(&mut self) -> Result<CoreStep<Self::RuleWitness>, RunError>;

    /// Builds a non-terminal applied transition.
    fn applied(self, step: StepCount, rule: Self::RuleWitness) -> Self::Transition;

    /// Builds a terminal return transition.
    fn returned(
        self,
        step: StepCount,
        rule: Self::RuleWitness,
        output: ReturnOutput,
    ) -> Self::Transition;

    /// Builds a terminal stable transition.
    fn stable(self, steps: StepCount) -> Self::Transition;

    /// Builds a terminal failed transition.
    fn failed(self, error: RunError) -> Self::Transition;

    /// Advances by one matching rule and maps the core result into public typestates.
    fn step_transition(mut self) -> Self::Transition {
        match self.session_step() {
            Ok(CoreStep::Applied(CoreAppliedRule::Rewrite { step, rule })) => {
                self.applied(step, rule)
            }
            Ok(CoreStep::Applied(CoreAppliedRule::Return { step, rule, output })) => {
                self.returned(step, rule, output)
            }
            Ok(CoreStep::Stable(steps)) => self.stable(steps),
            Err(error) => self.failed(error),
        }
    }
}

/// Shared transition construction for rule-attempt stepwise sessions.
pub(super) trait RuleAttemptRunSession: Sized {
    /// Public transition produced by this session.
    type Transition;

    /// Rule witness carried by public attempt transitions.
    type RuleWitness;

    /// Advances the private rule-attempt session with the right witness boundary.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if stepping through the private rule-attempt session fails.
    fn session_step(&mut self) -> Result<CoreRuleAttempt<Self::RuleWitness>, RunError>;

    /// Builds a non-terminal missed-attempt transition.
    fn missed(
        self,
        attempt: RuleAttemptCount,
        miss: RuleMiss<Self::RuleWitness>,
    ) -> Self::Transition;

    /// Builds a non-terminal applied-attempt transition.
    fn applied(
        self,
        attempt: RuleAttemptCount,
        step: StepCount,
        rule: Self::RuleWitness,
    ) -> Self::Transition;

    /// Builds a terminal return transition.
    fn returned(
        self,
        attempt: RuleAttemptCount,
        step: StepCount,
        rule: Self::RuleWitness,
        output: ReturnOutput,
    ) -> Self::Transition;

    /// Builds a terminal stable transition.
    fn stable(
        self,
        attempts: RuleAttemptCount,
        steps: StepCount,
        stable_reason: RuleAttemptStableReason<Self::RuleWitness>,
    ) -> Self::Transition;

    /// Builds a terminal failed transition.
    fn failed(self, error: RunError) -> Self::Transition;

    /// Advances by one executable rule line and maps the core result into public typestates.
    fn step_transition(mut self) -> Self::Transition {
        match self.session_step() {
            Ok(CoreRuleAttempt::Missed { attempt, miss }) => self.missed(attempt, miss),
            Ok(CoreRuleAttempt::Applied {
                attempt,
                applied: CoreAppliedRule::Rewrite { step, rule },
            }) => self.applied(attempt, step, rule),
            Ok(CoreRuleAttempt::Applied {
                attempt,
                applied: CoreAppliedRule::Return { step, rule, output },
            }) => self.returned(attempt, step, rule, output),
            Ok(CoreRuleAttempt::Stable {
                attempts,
                steps,
                stable_reason,
            }) => self.stable(attempts, steps, stable_reason),
            Err(error) => self.failed(error),
        }
    }
}

impl<'program> StepwiseRunSession for BorrowedRunSession<'program> {
    type Transition = BorrowedStepTransition<'program>;
    type RuleWitness = RuleView<'program>;

    fn session_step(&mut self) -> Result<CoreStep<Self::RuleWitness>, RunError> {
        self.session.step_borrowed()
    }

    fn applied(self, step: StepCount, rule: Self::RuleWitness) -> Self::Transition {
        BorrowedStepTransition::Applied(BorrowedAppliedStep {
            step,
            rule,
            session: self,
        })
    }

    fn returned(
        self,
        step: StepCount,
        rule: Self::RuleWitness,
        output: ReturnOutput,
    ) -> Self::Transition {
        let Session { program, core: _ } = self.session;
        BorrowedStepTransition::Returned(BorrowedReturnedRun {
            step,
            rule,
            program: program.program,
            output,
        })
    }

    fn stable(self, steps: StepCount) -> Self::Transition {
        let Session { program, core } = self.session;
        BorrowedStepTransition::Stable(BorrowedStableRun {
            steps,
            program: program.program,
            core,
        })
    }

    fn failed(self, error: RunError) -> Self::Transition {
        BorrowedStepTransition::Failed(BorrowedFailedRun::new(error, self))
    }
}

impl StepwiseRunSession for OwnedRunSession {
    type Transition = OwnedStepTransition;
    type RuleWitness = OwnedRuleWitness;

    fn session_step(&mut self) -> Result<CoreStep<Self::RuleWitness>, RunError> {
        self.session.step_owned()
    }

    fn applied(self, step: StepCount, rule: Self::RuleWitness) -> Self::Transition {
        OwnedStepTransition::Applied(OwnedAppliedStep {
            step,
            rule,
            session: self,
        })
    }

    fn returned(
        self,
        step: StepCount,
        rule: Self::RuleWitness,
        output: ReturnOutput,
    ) -> Self::Transition {
        let (program, _core) = self.session.into_program_core();
        OwnedStepTransition::Returned(OwnedReturnedRun {
            step,
            rule,
            program,
            output,
        })
    }

    fn stable(self, steps: StepCount) -> Self::Transition {
        let (program, core) = self.session.into_program_core();
        OwnedStepTransition::Stable(OwnedStableRun {
            steps,
            program,
            core,
        })
    }

    fn failed(self, error: RunError) -> Self::Transition {
        OwnedStepTransition::Failed(OwnedFailedRun::new(error, self))
    }
}

impl<'program> RuleAttemptRunSession for BorrowedRuleAttemptSession<'program> {
    type Transition = BorrowedRuleAttemptTransition<'program>;
    type RuleWitness = RuleView<'program>;

    fn session_step(&mut self) -> Result<CoreRuleAttempt<Self::RuleWitness>, RunError> {
        self.session.step_borrowed()
    }

    fn missed(
        self,
        attempt: RuleAttemptCount,
        miss: RuleMiss<Self::RuleWitness>,
    ) -> Self::Transition {
        BorrowedRuleAttemptTransition::Missed(BorrowedMissedRuleAttempt {
            attempt,
            miss,
            session: self,
        })
    }

    fn applied(
        self,
        attempt: RuleAttemptCount,
        step: StepCount,
        rule: Self::RuleWitness,
    ) -> Self::Transition {
        BorrowedRuleAttemptTransition::Applied(BorrowedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            session: self,
        })
    }

    fn returned(
        self,
        attempt: RuleAttemptCount,
        step: StepCount,
        rule: Self::RuleWitness,
        output: ReturnOutput,
    ) -> Self::Transition {
        let AttemptSession {
            program,
            core: _,
            cursor: _,
            attempt_budget: _,
        } = self.session;
        BorrowedRuleAttemptTransition::Returned(BorrowedRuleAttemptReturnedRun {
            attempt,
            step,
            rule,
            program: program.program,
            output,
        })
    }

    fn stable(
        self,
        attempts: RuleAttemptCount,
        steps: StepCount,
        stable_reason: RuleAttemptStableReason<Self::RuleWitness>,
    ) -> Self::Transition {
        let AttemptSession {
            program,
            core,
            cursor: _,
            attempt_budget: _,
        } = self.session;
        BorrowedRuleAttemptTransition::Stable(BorrowedRuleAttemptStableRun {
            attempts,
            steps,
            stable_reason,
            program: program.program,
            core,
        })
    }

    fn failed(self, error: RunError) -> Self::Transition {
        BorrowedRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(error, self))
    }
}

impl RuleAttemptRunSession for OwnedRuleAttemptSession {
    type Transition = OwnedRuleAttemptTransition;
    type RuleWitness = OwnedRuleWitness;

    fn session_step(&mut self) -> Result<CoreRuleAttempt<Self::RuleWitness>, RunError> {
        self.session.step_owned()
    }

    fn missed(
        self,
        attempt: RuleAttemptCount,
        miss: RuleMiss<Self::RuleWitness>,
    ) -> Self::Transition {
        OwnedRuleAttemptTransition::Missed(OwnedMissedRuleAttempt {
            attempt,
            miss,
            session: self,
        })
    }

    fn applied(
        self,
        attempt: RuleAttemptCount,
        step: StepCount,
        rule: Self::RuleWitness,
    ) -> Self::Transition {
        OwnedRuleAttemptTransition::Applied(OwnedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            session: self,
        })
    }

    fn returned(
        self,
        attempt: RuleAttemptCount,
        step: StepCount,
        rule: Self::RuleWitness,
        output: ReturnOutput,
    ) -> Self::Transition {
        let (program, _core) = self.session.into_program_core();
        OwnedRuleAttemptTransition::Returned(OwnedRuleAttemptReturnedRun {
            attempt,
            step,
            rule,
            program,
            output,
        })
    }

    fn stable(
        self,
        attempts: RuleAttemptCount,
        steps: StepCount,
        stable_reason: RuleAttemptStableReason<Self::RuleWitness>,
    ) -> Self::Transition {
        let (program, core) = self.session.into_program_core();
        OwnedRuleAttemptTransition::Stable(OwnedRuleAttemptStableRun {
            attempts,
            steps,
            stable_reason,
            program,
            core,
        })
    }

    fn failed(self, error: RunError) -> Self::Transition {
        OwnedRuleAttemptTransition::Failed(OwnedRuleAttemptFailedRun::new(error, self))
    }
}
