use anchor_lang::prelude::*;

declare_id!("5t75hMEMtV5rEN7BuRu3pQfyu2yUc5DVMBFqvdLxoZN5");

/// Append-only registry of Merkle roots over Aval verdict records.
/// No instruction updates, closes or transfers anything: the log only grows.
#[program]
pub mod aval_registry {
    use super::*;

    pub fn init_registry(ctx: Context<InitRegistry>) -> Result<()> {
        let registry = &mut ctx.accounts.registry;
        registry.authority = ctx.accounts.authority.key();
        registry.next_batch = 0;
        registry.next_record = 0;
        registry.last_root = [0u8; 32];
        registry.bump = ctx.bumps.registry;
        Ok(())
    }

    pub fn anchor_batch(
        ctx: Context<AnchorBatch>,
        root: [u8; 32],
        first_record: u64,
        count: u32,
    ) -> Result<()> {
        require!(count >= 1, RegistryError::EmptyBatch);
        let registry = &mut ctx.accounts.registry;
        require!(first_record == registry.next_record, RegistryError::NonContiguous);
        let clock = Clock::get()?;
        let batch = &mut ctx.accounts.batch;
        batch.registry = registry.key();
        batch.index = registry.next_batch;
        batch.root = root;
        batch.first_record = first_record;
        batch.count = count;
        batch.slot = clock.slot;
        batch.unix_timestamp = clock.unix_timestamp;
        batch.bump = ctx.bumps.batch;
        registry.next_batch = registry.next_batch.checked_add(1).ok_or(RegistryError::NonContiguous)?;
        registry.next_record = registry
            .next_record
            .checked_add(count as u64)
            .ok_or(RegistryError::NonContiguous)?;
        registry.last_root = root;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitRegistry<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + Registry::INIT_SPACE,
        seeds = [b"registry", authority.key().as_ref()],
        bump
    )]
    pub registry: Account<'info, Registry>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AnchorBatch<'info> {
    // No seeds check here on purpose: a wrong signer must fail with Unauthorized,
    // not with a seeds mismatch. Account<> already checks owner + discriminator.
    #[account(mut, has_one = authority @ RegistryError::Unauthorized)]
    pub registry: Account<'info, Registry>,
    #[account(
        init,
        payer = authority,
        space = 8 + Batch::INIT_SPACE,
        seeds = [b"batch", registry.key().as_ref(), &registry.next_batch.to_le_bytes()],
        bump
    )]
    pub batch: Account<'info, Batch>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct Registry {
    pub authority: Pubkey,
    pub next_batch: u64,
    pub next_record: u64,
    pub last_root: [u8; 32],
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Batch {
    pub registry: Pubkey,
    pub index: u64,
    pub root: [u8; 32],
    pub first_record: u64,
    pub count: u32,
    pub slot: u64,
    pub unix_timestamp: i64,
    pub bump: u8,
}

#[error_code]
pub enum RegistryError {
    #[msg("signer is not the registry authority")]
    Unauthorized,
    #[msg("batch does not start at the registry's next record")]
    NonContiguous,
    #[msg("a batch must contain at least one record")]
    EmptyBatch,
}
