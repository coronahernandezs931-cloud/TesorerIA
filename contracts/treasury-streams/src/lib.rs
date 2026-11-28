#![no_std]
#![allow(deprecated)]
//! TreasuryStreams — motor corporativo de nómina continua, fianzas, factoraje,
//! provisiones fiscales SAT/IMSS/PTU y cartas de crédito sobre Stellar Soroban.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env, String, Vec,
};

const INSTANCE_TTL: u32 = 518_400;
const PERSIST_TTL: u32 = 3_110_400;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StreamConfig {
    pub admin: Address,
    pub operator: Address,
    pub token: Address,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SalaryStream {
    pub id: u64,
    pub worker: Address,
    pub remaining: i128,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BondRecord {
    pub id: u64,
    pub supplier: Address,
    pub amount: i128,
    pub order_ref: String,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct WarrantyHold {
    pub id: u64,
    pub supplier: Address,
    pub holdback: i128,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct FactoringRecord {
    pub id: u64,
    pub supplier: Address,
    pub face_amount: i128,
    pub repaid: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SubBudget {
    pub id: u64,
    pub head: Address,
    pub remaining: i128,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ViaticoTicket {
    pub id: u64,
    pub employee: Address,
    pub disbursed: i128,
    pub closed: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LetterOfCredit {
    pub id: u64,
    pub beneficiary: Address,
    pub amount: i128,
    pub settled: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HedgePosition {
    pub id: u64,
    pub notional: i128,
    pub margin: i128,
    pub settled: bool,
}

#[contracttype]
pub enum DataKey {
    Config,
    NextId,
    PayrollPool,
    Stream(u64),
    Bond(u64),
    Warranty(u64),
    Factoring(u64),
    Budget(u64),
    Viatico(u64),
    LetterCredit(u64),
    Hedge(u64),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StreamError {
    InvalidAmount = 1,
    InvalidBps = 2,
    NotFound = 3,
    Inactive = 4,
    InsufficientPool = 5,
    EmptyList = 6,
}

#[contract]
pub struct TreasuryStreams;

fn cfg(env: &Env) -> StreamConfig {
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

fn pay_out(env: &Env, c: &StreamConfig, to: &Address, amount: i128) {
    if amount <= 0 {
        panic_with_error!(env, StreamError::InvalidAmount);
    }
    let tk = token::Client::new(env, &c.token);
    let me = env.current_contract_address();
    if tk.balance(&me) < amount {
        panic_with_error!(env, StreamError::InsufficientPool);
    }
    tk.transfer(&me, to, &amount);
}

#[contractimpl]
impl TreasuryStreams {
    pub fn __constructor(env: Env, admin: Address, operator: Address, token: Address) {
        env.storage().instance().set(&DataKey::Config, &StreamConfig { admin, operator, token });
        env.storage().instance().set(&DataKey::NextId, &1u64);
        bump(&env);
    }
    // __MODULES_ANCHOR__
}

#[cfg(test)]
mod test;
