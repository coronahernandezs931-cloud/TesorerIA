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
    // __ANCHOR__
}

#[cfg(test)]
mod test;
