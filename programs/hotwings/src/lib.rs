use anchor_lang::prelude::*;
use anchor_spl::token::*;
use crate::states::*; 

pub mod states;
pub mod consts;

declare_id!("6vxBssG3FvWset4jv3STQGGnq3mTqkkD2BSbYC5s7j89");

#[program]
pub mod hotwings {
    use anchor_spl::token;

    use super::*;

    pub fn initialize_lock_accounts(
        ctx: Context<InitializeLockAccounts>,
        users: Vec<InvestorInfo>, // Batch of users
    ) -> Result<()> {
        let lock_pool = &mut ctx.accounts.lock_pool_account;

        for user in users.iter() {
            // Step 1: Transfer tokens to the shared lock pool token account
            let cpi_accounts = Transfer {
                from: ctx.accounts.source_wallet.to_account_info(), // Admin's source wallet
                to: ctx.accounts.lock_pool_token_account.to_account_info(), // Centralized lock pool
                authority: ctx.accounts.admin_wallet.to_account_info(), // Admin wallet signature
            };
            let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
            token::transfer(cpi_ctx, user.token_amount)?;

            // Step 2: Add user locking information to LockPoolState
            let new_user_lock_info = UserLockInfo {
                user_wallet: user.wallet_address,
                total_tokens: user.token_amount,
                unlocked_tokens: 0, // Start with 0 unlocked tokens
                locked_tokens: user.token_amount,
            };

            // Push the user into the lock pool state and update total locked
            lock_pool.users.push(new_user_lock_info);
            lock_pool.total_locked += user.token_amount;
        }
    
        Ok(())
    }


}

