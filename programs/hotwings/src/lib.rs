use anchor_lang::prelude::*;
use anchor_spl::token::*;
use anchor_spl::token::{Token, TokenAccount};


declare_id!("6vxBssG3FvWset4jv3STQGGnq3mTqkkD2BSbYC5s7j89");

fn milestone_percentage(market_cap: u64) -> u8 {
    if market_cap >= 2_500_000 {
        return 100; // If the market cap exceeds or equals the last milestone, unlock 100%
    } else if market_cap >= 1_574_000 {
        return 70; // Milestone 7
    } else if market_cap >= 997_000 {
        return 60; // Milestone 6
    } else if market_cap >= 650_000 {
        return 50; // Milestone 5
    } else if market_cap >= 395_000 {
        return 40; // Milestone 4
    } else if market_cap >= 225_000 {
        return 30; // Milestone 3
    } else if market_cap >= 105_500 {
        return 20; // Milestone 2
    } else if market_cap >= 45_000 {
        return 10; // Milestone 1
    }
    0 // If market cap is below the first milestone, no tokens are unlocked
}

#[program]
pub mod hotwings {
    use anchor_spl::token;

    use super::*;

    pub fn initialize_lock_accounts(
        ctx: Context<InitializeLockAccounts>,
        users: Vec<InvestorInfo>, // Batch of users
    ) -> Result<()> {
        let lock_pool = &mut ctx.accounts.lock_pool_account;

         // Step 1: Initialize start_time if not already set
        if lock_pool.start_time == 0 {
            // Fetch current cluster time
            let clock = Clock::get()?; // Gets the current clock (cluster time)
            lock_pool.start_time = clock.unix_timestamp; // Set `start_time` using Solana clock
        }

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

    pub fn unlock_tokens(ctx: Context<UnlockTokens>, market_cap: u64) -> Result<()> {
        let lock_pool = &mut ctx.accounts.lock_pool_account;

        // Ensure the `admin_wallet` is the authorized signer
        let admin = &ctx.accounts.admin_wallet;
        require!(admin.is_signer, CustomError::Unauthorized); // Check if the admin is the signer
    
        // Fetch current milestone percentage
        let percentage = milestone_percentage(market_cap);
        let current_milestone = lock_pool.current_milestone;
    
        // Ensure we don’t process the same milestone multiple times
        require!(percentage > current_milestone * 10, CustomError::MilestoneNotReached);
    
        for user in lock_pool.users.iter_mut() {
            let total_to_unlock = user.total_tokens * percentage as u64 / 100;
            let newly_unlocked = total_to_unlock - user.unlocked_tokens;
    
            user.unlocked_tokens = total_to_unlock;    // Update unlocked tokens state
            user.locked_tokens -= newly_unlocked;     // Reduce locked tokens
    
            // Transfer unlocked tokens to the user
            let cpi_accounts = Transfer {
                from: ctx.accounts.lock_pool_token_account.to_account_info(),
                to: user.user_wallet.to_account_info(),
                authority: ctx.accounts.pda.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            token::transfer(cpi_ctx, newly_unlocked)?;
        }
    
        // Update current milestone
        lock_pool.current_milestone = (percentage / 10) as u8;
    
        Ok(())
    }


}


#[derive(Accounts)]
pub struct InitializeLockAccounts<'info> {
    #[account(mut)]
    pub lock_pool_account: Account<'info, LockPoolState>, // Track all locking data for users
    #[account(mut)]
    pub lock_pool_token_account: Account<'info, TokenAccount>, // Shared lock vault (PDA-owned)
    #[account(mut)]
    pub source_wallet: Account<'info, TokenAccount>, // Admin's funding source wallet
    #[account(mut)]
    pub admin_wallet: Signer<'info>, // Wallet signing token transfers (Presale Manager)
    pub token_program: Program<'info, Token>, // Standard SPL Token program
    pub clock: Sysvar<'info, Clock>, // Add the SysvarClock system account to fetch cluster time
}

#[derive(Clone, AnchorSerialize, AnchorDeserialize, Debug)]
pub struct InvestorInfo {
    pub wallet_address: Pubkey,
    pub token_amount: u64,
}

#[account]
pub struct LockPoolState {
    pub total_locked: u64,               // Total locked tokens in the pool
    pub users: Vec<UserLockInfo>,       // List of all users and locked info
    pub start_time: i64,  
    pub current_milestone: u8, 
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct UserLockInfo {
    pub user_wallet: Pubkey,            // Wallet address of the user
    pub total_tokens: u64,              // Purchased tokens during presale
    pub unlocked_tokens: u64,           // Unlocked tokens (via milestones)
    pub locked_tokens: u64,             // Remaining locked tokens
}

#[derive(Accounts)]
pub struct UnlockTokens<'info> {
    #[account(mut)]
    pub lock_pool_account: Account<'info, LockPoolState>, // Global LockPoolState (tracks locking state across users)

    #[account(mut)]
    pub lock_pool_token_account: Account<'info, TokenAccount>, // PDA-controlled SPL token account (the lock pool)

    /// CHECK: The PDA account, which acts as the authority for the LockPoolTokenAccount.
    pub pda: AccountInfo<'info>, // Program Derived Address (authority of LockPoolTokenAccount)

    #[account(mut)]
    pub admin_wallet: Signer<'info>, // ADMIN WALLET to trigger the unlocking process

    pub token_program: Program<'info, Token>, // SPL Token program for token transfers
}

#[error_code]
pub enum CustomError {
    #[msg("You are not authorized to call this instruction.")]
    Unauthorized,
    #[msg("Market cap is not reached")]
    MilestoneNotReached,
    // #[msg("Max hold amount exceeded")]
    // MaxHoldExceeded,
    // #[msg("Max supply amount exceeded")]
    // SupplyExceeded,
    // #[msg("Three months have not yet passed since the token distribution.")]
    // ThreeMonthsNotPassed,
}