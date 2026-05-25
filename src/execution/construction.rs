use crate::error::RunError;
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::program::{Program, ReturnOutput};

use super::attempt::{RuleAttemptStableReason, RuleMiss};
use super::engine::{AttemptSession, CoreAppliedRule, CoreRuleAttempt, CoreStep, RunCore, Session};
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

/// Data committed by one non-terminal stepwise rewrite.
pub(super) struct StepwiseApplied<RuleWitness> {
    /// Step number committed by this transition.
    step: StepCount,
    /// Rule witness selected before runtime side effects committed.
    rule: RuleWitness,
}

/// Data committed by a terminal stepwise return.
pub(super) struct StepwiseReturned<RuleWitness> {
    /// Step number that executed the return action.
    step: StepCount,
    /// Return rule witness selected before runtime side effects committed.
    rule: RuleWitness,
    /// Materialized return output.
    output: ReturnOutput,
}

/// Program ownership and terminal runtime core after a session is consumed.
pub(super) struct TerminalRunParts<ProgramHandle> {
    /// Parsed program retained by the terminal transition.
    program: ProgramHandle,
    /// Terminal runtime core retained when the terminal transition exposes state.
    core: RunCore,
}

/// Data committed by one non-applying rule attempt.
pub(super) struct RuleAttemptMissed<RuleWitness> {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Non-applying rule information.
    miss: RuleMiss<RuleWitness>,
}

/// Data committed by one non-terminal rule-attempt rewrite.
pub(super) struct RuleAttemptApplied<RuleWitness> {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Step number committed by this transition.
    step: StepCount,
    /// Rule witness selected before runtime side effects committed.
    rule: RuleWitness,
}

/// Data committed by a terminal rule-attempt return.
pub(super) struct RuleAttemptReturned<RuleWitness> {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Step number that executed the return action.
    step: StepCount,
    /// Return rule witness selected before runtime side effects committed.
    rule: RuleWitness,
    /// Materialized return output.
    output: ReturnOutput,
}

/// Data committed by a terminal stable rule-attempt run.
pub(super) struct RuleAttemptStable<RuleWitness> {
    /// Rule attempts consumed before stability.
    attempts: RuleAttemptCount,
    /// Rewrite steps committed before stability.
    steps: StepCount,
    /// Why the rule-attempt pass reached stability.
    stable_reason: RuleAttemptStableReason<RuleWitness>,
}

/// Public stepwise transition construction for one session ownership mode.
pub(super) trait StepwiseTransition<Session, RuleWitness, ProgramHandle> {
    /// Builds a non-terminal applied transition.
    fn applied(applied: StepwiseApplied<RuleWitness>, session: Session) -> Self;

    /// Builds a terminal return transition.
    fn returned(
        returned: StepwiseReturned<RuleWitness>,
        terminal: TerminalRunParts<ProgramHandle>,
    ) -> Self;

    /// Builds a terminal stable transition.
    fn stable(steps: StepCount, terminal: TerminalRunParts<ProgramHandle>) -> Self;

    /// Builds a terminal failed transition.
    fn failed(error: RunError, session: Session) -> Self;
}

/// Public rule-attempt transition construction for one session ownership mode.
pub(super) trait RuleAttemptTransition<Session, RuleWitness, ProgramHandle> {
    /// Builds a non-terminal missed-attempt transition.
    fn missed(missed: RuleAttemptMissed<RuleWitness>, session: Session) -> Self;

    /// Builds a non-terminal applied-attempt transition.
    fn applied(applied: RuleAttemptApplied<RuleWitness>, session: Session) -> Self;

    /// Builds a terminal return transition.
    fn returned(
        returned: RuleAttemptReturned<RuleWitness>,
        terminal: TerminalRunParts<ProgramHandle>,
    ) -> Self;

    /// Builds a terminal stable transition.
    fn stable(
        stable: RuleAttemptStable<RuleWitness>,
        terminal: TerminalRunParts<ProgramHandle>,
    ) -> Self;

    /// Builds a terminal failed transition.
    fn failed(error: RunError, session: Session) -> Self;
}

/// Shared transition construction for ordinary stepwise sessions.
pub(super) trait StepwiseRunSession: Sized {
    /// Public transition produced by this session.
    type Transition: StepwiseTransition<Self, Self::RuleWitness, Self::TerminalProgram>;

    /// Rule witness carried by public applied and returned transitions.
    type RuleWitness;

    /// Program handle retained by terminal public transitions.
    type TerminalProgram;

    /// Advances the private runtime session with the right witness boundary.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if stepping through the private runtime session fails.
    fn session_step(&mut self) -> Result<CoreStep<Self::RuleWitness>, RunError>;

    /// Consumes the public session into terminal program/core ownership.
    fn into_terminal_parts(self) -> TerminalRunParts<Self::TerminalProgram>;

    /// Advances by one matching rule and maps the core result into public typestates.
    fn step_transition(mut self) -> Self::Transition {
        match self.session_step() {
            Ok(CoreStep::Applied(CoreAppliedRule::Rewrite { step, rule })) => {
                Self::Transition::applied(StepwiseApplied { step, rule }, self)
            }
            Ok(CoreStep::Applied(CoreAppliedRule::Return { step, rule, output })) => {
                Self::Transition::returned(
                    StepwiseReturned { step, rule, output },
                    self.into_terminal_parts(),
                )
            }
            Ok(CoreStep::Stable(steps)) => {
                Self::Transition::stable(steps, self.into_terminal_parts())
            }
            Err(error) => Self::Transition::failed(error, self),
        }
    }
}

/// Shared transition construction for rule-attempt stepwise sessions.
pub(super) trait RuleAttemptRunSession: Sized {
    /// Public transition produced by this session.
    type Transition: RuleAttemptTransition<Self, Self::RuleWitness, Self::TerminalProgram>;

    /// Rule witness carried by public attempt transitions.
    type RuleWitness;

    /// Program handle retained by terminal public transitions.
    type TerminalProgram;

    /// Advances the private rule-attempt session with the right witness boundary.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if stepping through the private rule-attempt session fails.
    fn session_step(&mut self) -> Result<CoreRuleAttempt<Self::RuleWitness>, RunError>;

    /// Consumes the public session into terminal program/core ownership.
    fn into_terminal_parts(self) -> TerminalRunParts<Self::TerminalProgram>;

    /// Advances by one executable rule line and maps the core result into public typestates.
    fn step_transition(mut self) -> Self::Transition {
        match self.session_step() {
            Ok(CoreRuleAttempt::Missed { attempt, miss }) => {
                Self::Transition::missed(RuleAttemptMissed { attempt, miss }, self)
            }
            Ok(CoreRuleAttempt::Applied {
                attempt,
                applied: CoreAppliedRule::Rewrite { step, rule },
            }) => Self::Transition::applied(
                RuleAttemptApplied {
                    attempt,
                    step,
                    rule,
                },
                self,
            ),
            Ok(CoreRuleAttempt::Applied {
                attempt,
                applied: CoreAppliedRule::Return { step, rule, output },
            }) => Self::Transition::returned(
                RuleAttemptReturned {
                    attempt,
                    step,
                    rule,
                    output,
                },
                self.into_terminal_parts(),
            ),
            Ok(CoreRuleAttempt::Stable {
                attempts,
                steps,
                stable_reason,
            }) => Self::Transition::stable(
                RuleAttemptStable {
                    attempts,
                    steps,
                    stable_reason,
                },
                self.into_terminal_parts(),
            ),
            Err(error) => Self::Transition::failed(error, self),
        }
    }
}

impl<'program> StepwiseRunSession for BorrowedRunSession<'program> {
    type Transition = BorrowedStepTransition<'program>;
    type RuleWitness = RuleView<'program>;
    type TerminalProgram = &'program Program;

    fn session_step(&mut self) -> Result<CoreStep<Self::RuleWitness>, RunError> {
        self.session.step_borrowed()
    }

    fn into_terminal_parts(self) -> TerminalRunParts<Self::TerminalProgram> {
        let Session { program, core } = self.session;
        TerminalRunParts {
            program: program.program,
            core,
        }
    }
}

impl<'program>
    StepwiseTransition<BorrowedRunSession<'program>, RuleView<'program>, &'program Program>
    for BorrowedStepTransition<'program>
{
    fn applied(
        StepwiseApplied { step, rule }: StepwiseApplied<RuleView<'program>>,
        session: BorrowedRunSession<'program>,
    ) -> Self {
        BorrowedStepTransition::Applied(BorrowedAppliedStep {
            step,
            rule,
            session,
        })
    }

    fn returned(
        StepwiseReturned { step, rule, output }: StepwiseReturned<RuleView<'program>>,
        TerminalRunParts { program, core: _ }: TerminalRunParts<&'program Program>,
    ) -> Self {
        BorrowedStepTransition::Returned(BorrowedReturnedRun {
            step,
            rule,
            program,
            output,
        })
    }

    fn stable(
        steps: StepCount,
        TerminalRunParts { program, core }: TerminalRunParts<&'program Program>,
    ) -> Self {
        BorrowedStepTransition::Stable(BorrowedStableRun {
            steps,
            program,
            core,
        })
    }

    fn failed(error: RunError, session: BorrowedRunSession<'program>) -> Self {
        BorrowedStepTransition::Failed(BorrowedFailedRun::new(error, session))
    }
}

impl StepwiseRunSession for OwnedRunSession {
    type Transition = OwnedStepTransition;
    type RuleWitness = OwnedRuleWitness;
    type TerminalProgram = Program;

    fn session_step(&mut self) -> Result<CoreStep<Self::RuleWitness>, RunError> {
        self.session.step_owned()
    }

    fn into_terminal_parts(self) -> TerminalRunParts<Self::TerminalProgram> {
        let (program, core) = self.session.into_program_core();
        TerminalRunParts { program, core }
    }
}

impl StepwiseTransition<OwnedRunSession, OwnedRuleWitness, Program> for OwnedStepTransition {
    fn applied(
        StepwiseApplied { step, rule }: StepwiseApplied<OwnedRuleWitness>,
        session: OwnedRunSession,
    ) -> Self {
        OwnedStepTransition::Applied(OwnedAppliedStep {
            step,
            rule,
            session,
        })
    }

    fn returned(
        StepwiseReturned { step, rule, output }: StepwiseReturned<OwnedRuleWitness>,
        TerminalRunParts { program, core: _ }: TerminalRunParts<Program>,
    ) -> Self {
        OwnedStepTransition::Returned(OwnedReturnedRun {
            step,
            rule,
            program,
            output,
        })
    }

    fn stable(
        steps: StepCount,
        TerminalRunParts { program, core }: TerminalRunParts<Program>,
    ) -> Self {
        OwnedStepTransition::Stable(OwnedStableRun {
            steps,
            program,
            core,
        })
    }

    fn failed(error: RunError, session: OwnedRunSession) -> Self {
        OwnedStepTransition::Failed(OwnedFailedRun::new(error, session))
    }
}

impl<'program> RuleAttemptRunSession for BorrowedRuleAttemptSession<'program> {
    type Transition = BorrowedRuleAttemptTransition<'program>;
    type RuleWitness = RuleView<'program>;
    type TerminalProgram = &'program Program;

    fn session_step(&mut self) -> Result<CoreRuleAttempt<Self::RuleWitness>, RunError> {
        self.session.step_borrowed()
    }

    fn into_terminal_parts(self) -> TerminalRunParts<Self::TerminalProgram> {
        let AttemptSession {
            program,
            core,
            cursor: _,
            attempt_budget: _,
        } = self.session;
        TerminalRunParts {
            program: program.program,
            core,
        }
    }
}

impl<'program>
    RuleAttemptTransition<
        BorrowedRuleAttemptSession<'program>,
        RuleView<'program>,
        &'program Program,
    > for BorrowedRuleAttemptTransition<'program>
{
    fn missed(
        RuleAttemptMissed { attempt, miss }: RuleAttemptMissed<RuleView<'program>>,
        session: BorrowedRuleAttemptSession<'program>,
    ) -> Self {
        BorrowedRuleAttemptTransition::Missed(BorrowedMissedRuleAttempt {
            attempt,
            miss,
            session,
        })
    }

    fn applied(
        RuleAttemptApplied {
            attempt,
            step,
            rule,
        }: RuleAttemptApplied<RuleView<'program>>,
        session: BorrowedRuleAttemptSession<'program>,
    ) -> Self {
        BorrowedRuleAttemptTransition::Applied(BorrowedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            session,
        })
    }

    fn returned(
        RuleAttemptReturned {
            attempt,
            step,
            rule,
            output,
        }: RuleAttemptReturned<RuleView<'program>>,
        TerminalRunParts { program, core: _ }: TerminalRunParts<&'program Program>,
    ) -> Self {
        BorrowedRuleAttemptTransition::Returned(BorrowedRuleAttemptReturnedRun {
            attempt,
            step,
            rule,
            program,
            output,
        })
    }

    fn stable(
        RuleAttemptStable {
            attempts,
            steps,
            stable_reason,
        }: RuleAttemptStable<RuleView<'program>>,
        TerminalRunParts { program, core }: TerminalRunParts<&'program Program>,
    ) -> Self {
        BorrowedRuleAttemptTransition::Stable(BorrowedRuleAttemptStableRun {
            attempts,
            steps,
            stable_reason,
            program,
            core,
        })
    }

    fn failed(error: RunError, session: BorrowedRuleAttemptSession<'program>) -> Self {
        BorrowedRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(error, session))
    }
}

impl RuleAttemptRunSession for OwnedRuleAttemptSession {
    type Transition = OwnedRuleAttemptTransition;
    type RuleWitness = OwnedRuleWitness;
    type TerminalProgram = Program;

    fn session_step(&mut self) -> Result<CoreRuleAttempt<Self::RuleWitness>, RunError> {
        self.session.step_owned()
    }

    fn into_terminal_parts(self) -> TerminalRunParts<Self::TerminalProgram> {
        let (program, core) = self.session.into_program_core();
        TerminalRunParts { program, core }
    }
}

impl RuleAttemptTransition<OwnedRuleAttemptSession, OwnedRuleWitness, Program>
    for OwnedRuleAttemptTransition
{
    fn missed(
        RuleAttemptMissed { attempt, miss }: RuleAttemptMissed<OwnedRuleWitness>,
        session: OwnedRuleAttemptSession,
    ) -> Self {
        OwnedRuleAttemptTransition::Missed(OwnedMissedRuleAttempt {
            attempt,
            miss,
            session,
        })
    }

    fn applied(
        RuleAttemptApplied {
            attempt,
            step,
            rule,
        }: RuleAttemptApplied<OwnedRuleWitness>,
        session: OwnedRuleAttemptSession,
    ) -> Self {
        OwnedRuleAttemptTransition::Applied(OwnedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            session,
        })
    }

    fn returned(
        RuleAttemptReturned {
            attempt,
            step,
            rule,
            output,
        }: RuleAttemptReturned<OwnedRuleWitness>,
        TerminalRunParts { program, core: _ }: TerminalRunParts<Program>,
    ) -> Self {
        OwnedRuleAttemptTransition::Returned(OwnedRuleAttemptReturnedRun {
            attempt,
            step,
            rule,
            program,
            output,
        })
    }

    fn stable(
        RuleAttemptStable {
            attempts,
            steps,
            stable_reason,
        }: RuleAttemptStable<OwnedRuleWitness>,
        TerminalRunParts { program, core }: TerminalRunParts<Program>,
    ) -> Self {
        OwnedRuleAttemptTransition::Stable(OwnedRuleAttemptStableRun {
            attempts,
            steps,
            stable_reason,
            program,
            core,
        })
    }

    fn failed(error: RunError, session: OwnedRuleAttemptSession) -> Self {
        OwnedRuleAttemptTransition::Failed(OwnedRuleAttemptFailedRun::new(error, session))
    }
}
