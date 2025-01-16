use anchor_lang::prelude::*;

#[error_code]
pub enum CustomError {
    #[msg("Milestone Already Processed")]
    MilestoneAlreadyProcessed,

    #[msg("Three months have not yet passed")]

    ThreeMonthsNotPassed,
    #[msg("Invalid Pool PDA")]
    InvalidPoolPDA
}