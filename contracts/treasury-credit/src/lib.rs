#![no_std]
#![allow(deprecated)]
//! TreasuryCredit — líneas de crédito corporativo, tramos de tesorería CETES,
//! pedimentos aduanales IGI, M&A earnouts, dividendos con ISR y regalías en Soroban.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env, Vec,
};

const INSTANCE_TTL: u32 = 518_400;
const PERSIST_TTL: u32 = 3_110_400;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CreditConfig {
    pub admin: Address,
    pub operator: Address,
    pub token: Address,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LoanNote {
    pub id: u64,
    pub borrower: Address,
    pub outstanding: i128,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct YieldTranche {
    pub id: u64,
    pub custodian: Address,
    pub principal: i128,
    pub redeemed: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PedimentoEscrow {
    pub id: u64,
    pub duty_locked: i128,
    pub cleared: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct EarnoutEscrow {
    pub id: u64,
    pub seller: Address,
    pub holdback: i128,
    pub closed: bool,
}

#[contracttype]
pub enum DataKey {
    Config,
    NextId,
    FacilityPool,
    Loan(u64),
    Tranche(u64),
    Pedimento(u64),
    Earnout(u64),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CreditError {
    InvalidAmount = 1,
    InvalidBps = 2,
    NotFound = 3,
    Closed = 4,
    InsufficientLiquidity = 5,
    EmptyRecipients = 6,
}

#[contract]
pub struct TreasuryCredit;

fn cfg(env: &Env) -> CreditConfig {
    env.storage().instance().get(&DataKey::Config).unwrap()
}

fn bump(env: &Env) {
    env.storage().instance().extend_ttl(INSTANCE_TTL / 2, INSTANCE_TTL);
}

fn next_id(env: &Env) -> u64 {
    let id: u64 = env.storage().instance().get(&DataKey::NextId).unwrap_or(1);
    env.storage().instance().set(&DataKey::NextId, &(id + 1));
    id
}

fn save_p<K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>, V: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &K,
    val: &V,
) {
    env.storage().persistent().set(key, val);
    env.storage().persistent().extend_ttl(key, PERSIST_TTL / 2, PERSIST_TTL);
}

fn send_tokens(env: &Env, c: &CreditConfig, to: &Address, amount: i128) {
    if amount <= 0 {
        panic_with_error!(env, CreditError::InvalidAmount);
    }
    let tk = token::Client::new(env, &c.token);
    let me = env.current_contract_address();
    if tk.balance(&me) < amount {
        panic_with_error!(env, CreditError::InsufficientLiquidity);
    }
    tk.transfer(&me, to, &amount);
}

#[contractimpl]
impl TreasuryCredit {
    pub fn __constructor(env: Env, admin: Address, operator: Address, token: Address) {
        env.storage().instance().set(&DataKey::Config, &CreditConfig { admin, operator, token });
        env.storage().instance().set(&DataKey::NextId, &1u64);
        bump(&env);
    }

    // ---------- Bloque 1: Crédito Corporativo y Tesorería de Rendimiento ----------

    pub fn seed_credit_facility(env: Env, lender: Address, principal: i128) -> i128 {
        let c = cfg(&env);
        lender.require_auth();
        if principal <= 0 {
            panic_with_error!(&env, CreditError::InvalidAmount);
        }
        token::Client::new(&env, &c.token).transfer(&lender, &env.current_contract_address(), &principal);
        let pool = env.storage().instance().get(&DataKey::FacilityPool).unwrap_or(0i128) + principal;
        env.storage().instance().set(&DataKey::FacilityPool, &pool);
        bump(&env);
        env.events().publish((symbol_short!("fac_seed"), lender), principal);
        pool
    }

    pub fn draw_working_capital_loan(
        env: Env,
        borrower_unit: Address,
        principal: i128,
        origination_bps: u32,
        fee_vault: Address,
    ) -> LoanNote {
        let c = cfg(&env);
        c.admin.require_auth();
        if principal <= 0 || origination_bps == 0 || origination_bps > 1_000 {
            panic_with_error!(&env, CreditError::InvalidBps);
        }
        let fee = (principal * (origination_bps as i128)) / 10_000;
        let net_disbursed = principal - fee;
        send_tokens(&env, &c, &borrower_unit, net_disbursed);
        send_tokens(&env, &c, &fee_vault, fee);
        let id = next_id(&env);
        let note = LoanNote { id, borrower: borrower_unit.clone(), outstanding: principal, active: true };
        save_p(&env, &DataKey::Loan(id), &note);
        bump(&env);
        env.events().publish((symbol_short!("loan_drw"), borrower_unit), (id, net_disbursed, fee));
        note
    }

    pub fn amortize_loan_installment(
        env: Env,
        payer: Address,
        loan_id: u64,
        principal_part: i128,
        interest_part: i128,
        yield_vault: Address,
    ) -> LoanNote {
        let c = cfg(&env);
        payer.require_auth();
        let mut note: LoanNote = env
            .storage()
            .persistent()
            .get(&DataKey::Loan(loan_id))
            .unwrap_or_else(|| panic_with_error!(&env, CreditError::NotFound));
        if !note.active || principal_part <= 0 || interest_part <= 0 || principal_part > note.outstanding {
            panic_with_error!(&env, CreditError::InvalidAmount);
        }
        let tk = token::Client::new(&env, &c.token);
        tk.transfer(&payer, &env.current_contract_address(), &principal_part);
        tk.transfer(&payer, &yield_vault, &interest_part);
        note.outstanding -= principal_part;
        if note.outstanding == 0 {
            note.active = false;
        }
        save_p(&env, &DataKey::Loan(loan_id), &note);
        bump(&env);
        env.events().publish((symbol_short!("loan_amo"), payer), (loan_id, principal_part, interest_part));
        note
    }

    pub fn prepay_loan_with_rebate(env: Env, payer: Address, loan_id: u64, rebate_bps: u32) -> LoanNote {
        let c = cfg(&env);
        payer.require_auth();
        if rebate_bps > 1_500 {
            panic_with_error!(&env, CreditError::InvalidBps);
        }
        let mut note: LoanNote = env
            .storage()
            .persistent()
            .get(&DataKey::Loan(loan_id))
            .unwrap_or_else(|| panic_with_error!(&env, CreditError::NotFound));
        if !note.active || note.outstanding <= 0 {
            panic_with_error!(&env, CreditError::Closed);
        }
        let rebate = (note.outstanding * (rebate_bps as i128)) / 10_000;
        let payoff = note.outstanding - rebate;
        token::Client::new(&env, &c.token).transfer(&payer, &env.current_contract_address(), &payoff);
        note.outstanding = 0;
        note.active = false;
        save_p(&env, &DataKey::Loan(loan_id), &note);
        bump(&env);
        env.events().publish((symbol_short!("loan_pre"), payer), (loan_id, payoff));
        note
    }

    pub fn liquidate_defaulted_collateral(
        env: Env,
        recovery_vault: Address,
        seized_amount: i128,
    ) -> i128 {
        let c = cfg(&env);
        c.admin.require_auth();
        send_tokens(&env, &c, &recovery_vault, seized_amount);
        bump(&env);
        env.events().publish((symbol_short!("col_liq"), recovery_vault), seized_amount);
        seized_amount
    }

    pub fn subscribe_cetes_yield_tranche(
        env: Env,
        custodian_desk: Address,
        investment_amount: i128,
    ) -> YieldTranche {
        let c = cfg(&env);
        c.admin.require_auth();
        send_tokens(&env, &c, &custodian_desk, investment_amount);
        let id = next_id(&env);
        let tr = YieldTranche { id, custodian: custodian_desk.clone(), principal: investment_amount, redeemed: false };
        save_p(&env, &DataKey::Tranche(id), &tr);
        bump(&env);
        env.events().publish((symbol_short!("cetes_in"), custodian_desk), (id, investment_amount));
        tr
    }

    pub fn harvest_tranche_coupon(
        env: Env,
        payer: Address,
        tranche_id: u64,
        coupon_yield: i128,
        treasury_checking: Address,
    ) -> i128 {
        let c = cfg(&env);
        payer.require_auth();
        let tr: YieldTranche = env
            .storage()
            .persistent()
            .get(&DataKey::Tranche(tranche_id))
            .unwrap_or_else(|| panic_with_error!(&env, CreditError::NotFound));
        if tr.redeemed || coupon_yield <= 0 {
            panic_with_error!(&env, CreditError::InvalidAmount);
        }
        token::Client::new(&env, &c.token).transfer(&payer, &treasury_checking, &coupon_yield);
        bump(&env);
        env.events().publish((symbol_short!("coupon"), treasury_checking), (tranche_id, coupon_yield));
        coupon_yield
    }

    pub fn redeem_matured_tranche(env: Env, payer: Address, tranche_id: u64) -> YieldTranche {
        let c = cfg(&env);
        payer.require_auth();
        let mut tr: YieldTranche = env
            .storage()
            .persistent()
            .get(&DataKey::Tranche(tranche_id))
            .unwrap_or_else(|| panic_with_error!(&env, CreditError::NotFound));
        if tr.redeemed {
            panic_with_error!(&env, CreditError::Closed);
        }
        token::Client::new(&env, &c.token).transfer(&payer, &env.current_contract_address(), &tr.principal);
        tr.redeemed = true;
        save_p(&env, &DataKey::Tranche(tranche_id), &tr);
        bump(&env);
        env.events().publish((symbol_short!("cetes_ok"), payer), (tranche_id, tr.principal));
        tr
    }
    // __ANCHOR__
}

#[cfg(test)]
mod test;
