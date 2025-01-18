use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount, Transfer}; 

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

        // ✅ Security: Check if pool is already initialized
        require!(
            ctx.accounts.lock_pool_token_account.owner == ctx.accounts.pda.key(),
            CustomError::Unauthorized
        );
        require!(
            lock_pool.start_time == 0,
            CustomError::AlreadyInitialized
        );

         // Step 1: Initialize start_time if not already set
        if lock_pool.start_time == 0 {
            // Fetch current cluster time
            let clock = Clock::get()?; // Gets the current clock (cluster time)
            lock_pool.start_time = clock.unix_timestamp; // Set `start_time` using Solana clock
        }

        for user in users.iter() {
            // ✅ Security Check: Ensure token amount is valid
            require!(user.token_amount > 0, CustomError::InvalidTokenAmount);

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
        // ✅ Security Check: Ensure caller is admin
        require!(
            ctx.accounts.admin_wallet.key() == ctx.accounts.pda.key(),
            CustomError::Unauthorized
        );

        // Ensure the admin wallet is authorized
        let admin = &ctx.accounts.admin_wallet;
        require!(admin.is_signer, CustomError::Unauthorized); // Check if the admin is the signer
    
        // Fetch current milestone percentage
        let percentage = milestone_percentage(market_cap);
        let current_milestone = lock_pool.current_milestone;
    
        // Ensure we don’t process the same milestone multiple times
        require!(percentage > current_milestone * 10, CustomError::MilestoneNotReached);

        let percentage = milestone_percentage(market_cap);
        require!(percentage > 0, CustomError::MilestoneNotReached);
        
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

    pub fn full_unlock(ctx: Context<FullUnlock>) -> Result<()> {
        let lock_pool = &mut ctx.accounts.lock_pool_account;
    
        // Ensure that the full unlock has not been executed yet
        require!(!lock_pool.full_unlock_executed, CustomError::FullUnlockAlreadyExecuted);
    
        // Get the current Solana cluster time
        let current_time = ctx.accounts.clock.unix_timestamp;
    
        // Ensure that 3 months have passed since `start_time`
        require!(
            current_time >= lock_pool.start_time + (3 * 30 * 24 * 60 * 60), // 3 months in seconds
            CustomError::UnlockTooSoon
        );
    
        // Iterate over all users to unlock their remaining locked tokens
        for user in lock_pool.users.iter_mut() {
            let newly_unlocked_tokens = user.locked_tokens; // All remaining locked tokens
    
            // Check to avoid unnecessary processing
            if newly_unlocked_tokens > 0 {
                // Update user's token state
                user.unlocked_tokens += newly_unlocked_tokens;
                user.locked_tokens = 0;
    
                // Transfer all remaining locked tokens from lock pool account to the user's wallet
                let cpi_accounts = Transfer {
                    from: ctx.accounts.lock_pool_token_account.to_account_info(),
                    to: user.user_wallet.to_account_info(),
                    authority: ctx.accounts.pda.to_account_info(),
                };
                let cpi_program = ctx.accounts.token_program.to_account_info();
                let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
    
                token::transfer(cpi_ctx, newly_unlocked_tokens)?;
            }
        }
    
        // Mark full unlock as executed
        lock_pool.full_unlock_executed = true;
    
        Ok(())
    }

    pub fn purchase_tokens(ctx: Context<PurchaseTokens>, total_paid_tokens: u64) -> Result<()> {
        let lock_pool = &mut ctx.accounts.lock_pool_account;
    
        // Error 1: Ensure `total_paid_tokens` is greater than 0
        require!(total_paid_tokens > 0, CustomError::InvalidTokenAmount);
        // Determine the percentage of tokens to unlock immediately based on the current milestone
        let unlock_percentage = milestone_percentage_from_milestone(lock_pool.current_milestone);
    
        // Calculate unlocked and locked tokens
        let unlocked_tokens = total_paid_tokens * unlock_percentage as u64 / 100;
        let locked_tokens = total_paid_tokens - unlocked_tokens;

        // Error 2: Ensure the lock pool has enough tokens for the unlocked portion
        require!(
            ctx.accounts.lock_pool_token_account.amount >= unlocked_tokens,
            CustomError::InsufficientPoolBalance
        );
    
        // Handle unlocked tokens: Transfer `unlocked_tokens` directly to the user's wallet
        if unlocked_tokens > 0 {
            let cpi_accounts = Transfer {
                from: ctx.accounts.lock_pool_token_account.to_account_info(),
                to: ctx.accounts.user_token_account.to_account_info(),
                authority: ctx.accounts.pda.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
    
            token::transfer(cpi_ctx, unlocked_tokens)?;
        }
    
        
    
        // Handle locked tokens: Add locked tokens to the LockPoolState for this user
        if locked_tokens > 0 {
            // Step: Transfer tokens to the shared lock pool token account
            let cpi_accounts_lock_transfer = Transfer {
                from: ctx.accounts.token_pool_account.to_account_info(), // Source is Raydium token pool
                to: ctx.accounts.lock_pool_token_account.to_account_info(), // Destination is Lock Pool Token Account
                authority: ctx.accounts.pda.to_account_info(), // Authority is program-derived
            };
            let lock_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts_lock_transfer);
            token::transfer(lock_ctx, locked_tokens)?;

            // Check if the user is already in the LockPoolState
            if let Some(user) = lock_pool.users.iter_mut().find(|u| u.user_wallet == ctx.accounts.user_wallet.key()) {
                // Update existing user's locked tokens
                user.total_tokens += locked_tokens;
                user.locked_tokens += locked_tokens;
            } else {
                // Add new user entry to the LockPool
                lock_pool.users.push(UserLockInfo {
                    user_wallet: ctx.accounts.user_wallet.key(), // Buyer’s wallet
                    total_tokens: locked_tokens,                 // Total tokens purchased
                    locked_tokens,                              // Tokens still locked
                    unlocked_tokens: 0,                         // Tokens immediately unlocked
                });
            }
        }
    
        Ok(())
    }

    pub fn finalize_unlock(ctx: Context<FinalizeUnlock>) -> Result<()> {
        let lock_pool = &mut ctx.accounts.lock_pool_account;
        
        let clock = Clock::get()?; // Get Solana cluster time
        let current_time = clock.unix_timestamp;
    
        // Ensure unlock conditions are met: either final milestone or 3-month full unlock
        let unlock_condition_met = lock_pool.current_milestone >= 8
            || current_time >= lock_pool.start_time + (3 * 30 * 24 * 60 * 60); // 3 months
        require!(unlock_condition_met, CustomError::UnlockTooSoon);
    
        // Calculate 25% auto-sell amount
        let auto_sell_tokens = ctx
            .accounts
            .project_wallet
            .amount
            .checked_mul(25)
            .and_then(|v| v.checked_div(100))
            .ok_or(CustomError::InvalidTokenAmount)?;
    
        require!(auto_sell_tokens > 0, CustomError::InvalidTokenAmount);
    
        // Perform token transfer for auto-sell
        let cpi_accounts = Transfer {
            from: ctx.accounts.project_wallet.to_account_info(),
            to: ctx.accounts.dex_liquidity_wallet.to_account_info(),
            authority: ctx.accounts.project_wallet_authority.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, auto_sell_tokens)?;
    
        // Deactivate maximum hold limit
        lock_pool.is_max_hold_limit_active = false;
    
        Ok(())
    }


}

// =======================================================struct=============================================

#[derive(Accounts)]
pub struct TransferHookContext<'info> {
    #[account(mut)]
    pub source_wallet: Account<'info, TokenAccount>, // Source of the transfer
    #[account(mut)]
    pub destination_wallet: Account<'info, TokenAccount>, // Destination of the transfer
    #[account(mut)]
    pub burn_wallet: Account<'info, TokenAccount>, // Burn Wallet
    #[account(mut)]
    pub marketing_wallet: Account<'info, TokenAccount>, // Marketing Wallet
    /// CHECK: The transfer instruction (modifies amount)
    pub transfer_instruction: AccountInfo<'info>,
    pub authority: AccountInfo<'info>, // PDA for Token Authority
    pub token_program: Program<'info, Token>, // SPL Token program
    #[account(mut)]
    pub lock_pool_account: Account<'info, LockPoolState>, // Track all locking data for users
    #[account(mut)]
    pub lock_pool_token_account: Account<'info, TokenAccount>, // Shared lock vault (PDA-owned)
    pub user_wallet: Signer<'info>, // User wallet
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
    pub clock: Sysvar<'info, Clock>, // Fetch cluster time from SysvarClock
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
    pub full_unlock_executed: bool
    pub is_max_hold_limit_active: bool,  // NEW: Enable/Disable max hold restrictions
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

#[derive(Accounts)]
pub struct FullUnlock<'info> {
    #[account(mut)]
    pub lock_pool_account: Account<'info, LockPoolState>, // Global LockPoolState (tracks locking state across users)
    #[account(mut)]
    pub lock_pool_token_account: Account<'info, TokenAccount>, // PDA-controlled SPL token account (the lock pool)
    /// CHECK: PDA authority over the LockPool Token Account
    pub pda: AccountInfo<'info>, // Program Derived Address (authority of LockPoolTokenAccount)
    #[account(mut)]
    pub admin_wallet: Signer<'info>, // ADMIN WALLET to trigger the full unlock operation
    pub token_program: Program<'info, Token>, // SPL Token program for token transfers
    pub clock: Sysvar<'info, Clock>, // Solana Clock Sysvar to fetch current cluster time
}

#[derive(Accounts)]
pub struct PurchaseTokens<'info> {
    #[account(mut)]
    pub lock_pool_account: Account<'info, LockPoolState>, // Global LockPoolState
    #[account(mut)]
    pub lock_pool_token_account: Account<'info, TokenAccount>, // PDA-controlled lock pool account
    /// CHECK: Program Derived Address (PDA) for authority over the LockPool
    pub pda: AccountInfo<'info>, // PDA authority for lock pool transfers
    #[account(mut)]
    pub user_wallet: Signer<'info>, // Buyer's wallet (receiving unlocked tokens)
    #[account(mut)]
    pub user_token_account: Account<'info, TokenAccount>, // Buyer's token account to receive unlocked tokens
    pub token_program: Program<'info, Token>, // SPL Token Program
}

#[derive(Accounts)]
pub struct FinalizeUnlock<'info> {
    #[account(mut)]
    pub lock_pool_account: Account<'info, LockPoolState>, // Global LockPoolState (tracks locking state across users)
    /// CHECK: Project wallet (source of auto-sales)
    #[account(mut)]
    pub project_wallet: Account<'info, TokenAccount>, // Project/Presale Manager's wallet
    /// CHECK: Authority over the `project_wallet`
    pub project_wallet_authority: Signer<'info>, // Authority to approve sales from the project wallet
    #[account(mut)]
    pub dex_liquidity_wallet: Account<'info, TokenAccount>, // Wallet or DEX account receiving the auto-sell tokens
    pub token_program: Program<'info, Token>, // SPL Token program for transfers
    pub clock: Sysvar<'info, Clock>, // Solana Clock Sysvar to fetch current cluster time
}


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

fn milestone_percentage_from_milestone(current_milestone: u8) -> u8 {
    match current_milestone {
        1 => 10, // Milestone 1: 10%
        2 => 20, // Milestone 2: 20%
        3 => 30, // Milestone 3: 30%
        4 => 40, // Milestone 4: 40%
        5 => 50, // Milestone 5: 50%
        6 => 60, // Milestone 6: 60%
        7 => 70, // Milestone 7: 70%
        8 => 100, // Milestone 8: 100%
        _ => 0,   // Fallback: No unlock for unknown milestones
    }
}

// =============================================TransferHook================================================

fn initialize_token_with_transfer_hook(
    payer: Pubkey,
    mint_authority: Pubkey,
    freeze_authority: Option<Pubkey>,
) -> Result<()> {
    let mint = Keypair::new(); // Generate a keypair for the mint

    // Specify the extensions to enable
    let extensions = vec![
        ExtensionType::TransferHook, // Enables the TransferHook logic
    ];

    // Create the mint with the above extensions
    let instruction = initialize_mint_with_extension(
        &spl_token_2022::id(),
        &mint.pubkey(),
        &payer,
        &mint_authority,
        freeze_authority.as_ref(),
        extensions, // TransferHook extension
    );

    // Send the transaction to initialize the mint
    let tx = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&payer),
        &[payer_signer],
        recent_blockhash,
    );
    banks_client.process_transaction(tx)?;

    Ok(())
}

pub fn process_transfer_hook(ctx: Context<TransferHookContext>) -> Result<()> {
    // Extract the program IDs for source and destination
    let source_program_id = ctx.accounts.source_wallet.owner(); // SPL Token account owning program ID
    let destination_program_id = ctx.accounts.destination_wallet.owner(); // SPL Token account owning program ID

    // Check if it's a DEX transaction
    let is_dex = is_dex_transaction(source_program_id, destination_program_id);

    if is_dex {
        // Tax Logic
        let transfer_amount = ctx.accounts.transfer_instruction.amount;
        let tax = transfer_amount * 15 / 1000; // 1.5% total tax
        let burn_amount = tax / 2; // 0.75% for Burn
        let marketing_amount = tax / 2; // 0.75% for Marketing Wallet
        let net_transfer = transfer_amount - tax;
        let lock_pool = &mut ctx.accounts.lock_pool_account;

        if lock_pool.is_max_hold_limit_active {
            let user_hold_amount = lock_pool.users.iter()
                .find(|u| u.user_wallet == ctx.accounts.user_wallet.key())
                .map_or(0, |u| u.total_tokens + net_transfer);
        
            require!(
                user_hold_amount <= MAX_HOLD_AMOUNT,
                CustomError::MaxHoldExceeded
            );
        }
    
        // Burn tokens
        let cpi_ctx_burn = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.source_wallet.to_account_info(),
                to: ctx.accounts.burn_wallet.to_account_info(),
                authority: ctx.accounts.authority.clone(),
            },
            ctx.signer_seeds, // PDA signer
        );
        token::transfer(cpi_ctx_burn, burn_amount)?;

        // Send tokens to marketing wallet
        let cpi_ctx_marketing = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.source_wallet.to_account_info(),
                to: ctx.accounts.marketing_wallet.to_account_info(),
                authority: ctx.accounts.authority.clone(),
            },
            ctx.signer_seeds, // PDA signer
        );
        token::transfer(cpi_ctx_marketing, marketing_amount)?;

        // Update the final amount to be transferred
        // ctx.accounts.transfer_instruction.amount = net_transfer;
        
        // Step: Transfer tokens to the shared lock pool token account
        let cpi_ctx_locking = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.source_wallet.to_account_info(),
                to: ctx.accounts.lock_pool_token_account.to_account_info(),
                authority: ctx.accounts.authority.clone(),
            },
            ctx.signer_seeds, // PDA signer
        );
        token::transfer(cpi_ctx_locking, net_transfer)?;
        

        // Check if the user is already in the LockPoolState
        if let Some(user) = lock_pool.users.iter_mut().find(|u| u.user_wallet == ctx.accounts.user_wallet.key()) {
            // Update existing user's locked tokens
            user.total_tokens += net_transfer;
            user.locked_tokens += net_transfer;
        } else {
            // Add new user entry to the LockPool
            lock_pool.users.push(UserLockInfo {
                user_wallet: ctx.accounts.user_wallet.key(), // Buyer’s wallet
                total_tokens: locked_tokens,                 // Total tokens purchased
                locked_tokens,                              // Tokens still locked
                unlocked_tokens: 0,                         // Tokens immediately unlocked
            });
        }

    }

    // Allow the transfer to proceed
    Ok(())
}
const MAX_HOLD_AMOUNT: u64 = 50_000_000;
const YOUR_PROJECT_WALLET: Pubkey = Pubkey::new_from_array([34o4N3JLTxGsqHtFqwpsPDRyimmhbGrUNhhro6xGKhAS]);
const YOUR_MARKET_WALLET: Pubkey = Pubkey::new_from_array([Fn3Co7FJyMHM6RpPD74TX4Ah2ShLhyNHzNie19jNg8BG]);
// Known DEX program IDs
const SERUM_DEX_PROGRAM_ID_DEV_1: Pubkey = Pubkey::new_from_array([DESVgJVGajEgKGXhb6XmqDHGz3VjdgP7rEVESBgxmroY]); ///DevNET
const SERUM_DEX_PROGRAM_ID_MAIN_1: Pubkey = Pubkey::new_from_array([9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin]); ///MainNET
const RAYDIUM_PROGRAM_ID_DEV_1: Pubkey = Pubkey::new_from_array([CPMDWBwJDtYax9qW7AyRuVC19Cc4L4Vcy4n2BHAbHkCW]);    ///DevNET
const RAYDIUM_PROGRAM_ID_DEV_2: Pubkey = Pubkey::new_from_array([HWy1jotHpo6UqeQxx49dpYYdQB8wj9Qk9MdxwjLvDHB8]);    ///DevNET
const RAYDIUM_PROGRAM_ID_DEV_3: Pubkey = Pubkey::new_from_array([DDg4VmQaJV9ogWce7LpcjBA9bv22wRp5uaTPa5pGjijF]);    ///DevNET
const RAYDIUM_PROGRAM_ID_DEV_4: Pubkey = Pubkey::new_from_array([devi51mZmdwUJGU9hjN27vEz64Gps7uUefqxg27EAtH]);    ///DevNET
const RAYDIUM_PROGRAM_ID_DEV_5: Pubkey = Pubkey::new_from_array([85BFyr98MbCUU9MVTEgzx1nbhWACbJqLzho6zd6DZcWL]);    ///DevNET
const RAYDIUM_PROGRAM_ID_DEV_6: Pubkey = Pubkey::new_from_array([EcLzTrNg9V7qhcdyXDe2qjtPkiGzDM2UbdRaeaadU5r2]);    ///DevNET
const RAYDIUM_PROGRAM_ID_DEV_7: Pubkey = Pubkey::new_from_array([BVChZ3XFEwTMUk1o9i3HAf91H6mFxSwa5X2wFAWhYPhU]);    ///DevNET
const RAYDIUM_PROGRAM_ID_MAIN_1: Pubkey = Pubkey::new_from_array([CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C]);    ///MainNET
const RAYDIUM_PROGRAM_ID_MAIN_2: Pubkey = Pubkey::new_from_array([675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8]);    ///MainNET
const RAYDIUM_PROGRAM_ID_MAIN_3: Pubkey = Pubkey::new_from_array([5quBtoiQqxF9Jv6KYKctB59NT3gtJD2Y65kdnB1Uev3h]);    ///MainNET
const RAYDIUM_PROGRAM_ID_MAIN_4: Pubkey = Pubkey::new_from_array([CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK]);    ///MainNET
const RAYDIUM_PROGRAM_ID_MAIN_5: Pubkey = Pubkey::new_from_array([routeUGWgWzqBWFcrCfv8tritsqukccJPu3q5GPP3xS]);    ///MainNET
const RAYDIUM_PROGRAM_ID_MAIN_6: Pubkey = Pubkey::new_from_array([EhhTKczWMGQt46ynNeRX1WfeagwwJd7ufHvCDjRxjo5Q]);    ///MainNET
const RAYDIUM_PROGRAM_ID_MAIN_7: Pubkey = Pubkey::new_from_array([9KEPoZmtHUrBbhWN1v1KWLMkkvwY6WLtAVUCPRtRjP4z]);    ///MainNET
const RAYDIUM_PROGRAM_ID_MAIN_8: Pubkey = Pubkey::new_from_array([FarmqiPv5eAj3j1GMdMCMUGXqPUvmquZtMy86QH6rzhG]);    ///MainNET
const RAYDIUM_PROGRAM_ID_MAIN_9: Pubkey = Pubkey::new_from_array([9HzJyW1qZsEiSfMUf6L2jo3CcTKAyBmSyKdwQeYisHrC]);    ///MainNET
const ORCA_PROGRAM_ID_MAIN_1: Pubkey = Pubkey::new_from_array([whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc]);    ///MainNET

// Helper function to determine if a transaction is associated with a DEX
fn is_dex_transaction(source_program_id: &Pubkey, destination_program_id: &Pubkey) -> bool {
    // Known DEX program IDs
    const KNOWN_DEX_PROGRAMS: [&Pubkey; 19] = [
        &SERUM_DEX_PROGRAM_ID_DEV_1,
        &SERUM_DEX_PROGRAM_ID_MAIN_1,
        &RAYDIUM_PROGRAM_ID_DEV_1,
        &RAYDIUM_PROGRAM_ID_DEV_2,
        &RAYDIUM_PROGRAM_ID_DEV_3,
        &RAYDIUM_PROGRAM_ID_DEV_4,
        &RAYDIUM_PROGRAM_ID_DEV_5,
        &RAYDIUM_PROGRAM_ID_DEV_6,
        &RAYDIUM_PROGRAM_ID_DEV_7,
        &RAYDIUM_PROGRAM_ID_MAIN_1,
        &RAYDIUM_PROGRAM_ID_MAIN_2,
        &RAYDIUM_PROGRAM_ID_MAIN_3,
        &RAYDIUM_PROGRAM_ID_MAIN_4,
        &RAYDIUM_PROGRAM_ID_MAIN_5,
        &RAYDIUM_PROGRAM_ID_MAIN_6,
        &RAYDIUM_PROGRAM_ID_MAIN_7,
        &RAYDIUM_PROGRAM_ID_MAIN_8,
        &RAYDIUM_PROGRAM_ID_MAIN_9,
        &ORCA_PROGRAM_ID_MAIN_1,
    ];

    // Check if source or destination is a known DEX program
    KNOWN_DEX_PROGRAMS.contains(source_program_id) || KNOWN_DEX_PROGRAMS.contains(destination_program_id)
}


// =====================================================Error=============================================


#[error_code]
pub enum CustomError {
    #[msg("Unauthorized: You do not have permission to call this method.")]
    Unauthorized,
    #[msg("Milestone has not been reached for token unlock.")]
    MilestoneNotReached,
    #[msg("Conditions not met: Final milestone or 3-month unlock period.")]
    UnlockTooSoon,
    #[msg("Invalid Token Amount")]
    InvalidTokenAmount,
    #[msg("Insufficient Pool Balance")]
    InsufficientPoolBalance,
    #[msg("Max hold amount exceeded")]
    MaxHoldExceeded,
    #[msg("Already Full Unlocked")]
    FullUnlockAlreadyExecuted,
    #[msg("Insufficient Pool Balance")]
    InsufficientPoolBalance,
    #[msg("Lock accounts have already been initialized.")]
    AlreadyInitialized,
}
