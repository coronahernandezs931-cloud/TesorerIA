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

    // ---------- Módulo 1: Nómina Continua y Prestaciones LFT ----------

    pub fn fund_payroll_pool(env: Env, funder: Address, amount: i128) -> i128 {
        let c = cfg(&env);
        funder.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, StreamError::InvalidAmount);
        }
        let tk = token::Client::new(&env, &c.token);
        tk.transfer(&funder, &env.current_contract_address(), &amount);
        let pool: i128 = env.storage().instance().get(&DataKey::PayrollPool).unwrap_or(0) + amount;
        env.storage().instance().set(&DataKey::PayrollPool, &pool);
        bump(&env);
        env.events().publish((symbol_short!("pool_in"), funder), amount);
        pool
    }

    pub fn open_salary_stream(
        env: Env,
        worker: Address,
        total_salary: i128,
        signing_bonus: i128,
    ) -> SalaryStream {
        let c = cfg(&env);
        c.operator.require_auth();
        if total_salary <= 0 || signing_bonus <= 0 {
            panic_with_error!(&env, StreamError::InvalidAmount);
        }
        pay_out(&env, &c, &worker, signing_bonus);
        let id = next_id(&env);
        let st = SalaryStream { id, worker: worker.clone(), remaining: total_salary, active: true };
        save_p(&env, &DataKey::Stream(id), &st);
        bump(&env);
        env.events().publish((symbol_short!("sal_open"), worker), (id, total_salary, signing_bonus));
        st
    }

    pub fn claim_accrued_salary(env: Env, stream_id: u64, payout_slice: i128) -> SalaryStream {
        let c = cfg(&env);
        c.operator.require_auth();
        let mut st: SalaryStream = env
            .storage()
            .persistent()
            .get(&DataKey::Stream(stream_id))
            .unwrap_or_else(|| panic_with_error!(&env, StreamError::NotFound));
        if !st.active || payout_slice <= 0 || payout_slice > st.remaining {
            panic_with_error!(&env, StreamError::InvalidAmount);
        }
        pay_out(&env, &c, &st.worker, payout_slice);
        st.remaining -= payout_slice;
        if st.remaining == 0 {
            st.active = false;
        }
        save_p(&env, &DataKey::Stream(stream_id), &st);
        bump(&env);
        env.events().publish((symbol_short!("sal_clm"), st.worker.clone()), (stream_id, payout_slice));
        st
    }

    pub fn disburse_aguinaldo_bonus(
        env: Env,
        worker: Address,
        monthly_salary: i128,
        worked_days: u32,
    ) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if monthly_salary <= 0 || worked_days == 0 || worked_days > 365 {
            panic_with_error!(&env, StreamError::InvalidAmount);
        }
        // LFT Art. 87: mínimo 15 días de salario diario proporcional a días trabajados
        let daily = monthly_salary / 30;
        let aguinaldo = (daily * 15 * (worked_days as i128)) / 365;
        pay_out(&env, &c, &worker, aguinaldo);
        bump(&env);
        env.events().publish((symbol_short!("aguinal"), worker), aguinaldo);
        aguinaldo
    }

    pub fn settle_severance_finiquito(
        env: Env,
        stream_id: u64,
        indemnity_extra: i128,
    ) -> i128 {
        let c = cfg(&env);
        c.admin.require_auth();
        if indemnity_extra < 0 {
            panic_with_error!(&env, StreamError::InvalidAmount);
        }
        let mut st: SalaryStream = env
            .storage()
            .persistent()
            .get(&DataKey::Stream(stream_id))
            .unwrap_or_else(|| panic_with_error!(&env, StreamError::NotFound));
        if !st.active {
            panic_with_error!(&env, StreamError::Inactive);
        }
        let total_finiquito = st.remaining + indemnity_extra;
        pay_out(&env, &c, &st.worker, total_finiquito);
        st.remaining = 0;
        st.active = false;
        save_p(&env, &DataKey::Stream(stream_id), &st);
        bump(&env);
        env.events().publish((symbol_short!("finiquit"), st.worker), (stream_id, total_finiquito));
        total_finiquito
    }

    // ---------- Módulo 2: Fianzas de Cumplimiento y Garantía de Obra ----------

    pub fn stake_supplier_bond(
        env: Env,
        funder: Address,
        supplier: Address,
        bond_amount: i128,
        order_ref: String,
    ) -> BondRecord {
        let c = cfg(&env);
        funder.require_auth();
        if bond_amount <= 0 {
            panic_with_error!(&env, StreamError::InvalidAmount);
        }
        token::Client::new(&env, &c.token).transfer(&funder, &env.current_contract_address(), &bond_amount);
        let id = next_id(&env);
        let b = BondRecord { id, supplier: supplier.clone(), amount: bond_amount, order_ref, active: true };
        save_p(&env, &DataKey::Bond(id), &b);
        bump(&env);
        env.events().publish((symbol_short!("bond_in"), supplier), (id, bond_amount));
        b
    }

    pub fn release_supplier_bond(env: Env, bond_id: u64) -> BondRecord {
        let c = cfg(&env);
        c.admin.require_auth();
        let mut b: BondRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bond(bond_id))
            .unwrap_or_else(|| panic_with_error!(&env, StreamError::NotFound));
        if !b.active {
            panic_with_error!(&env, StreamError::Inactive);
        }
        pay_out(&env, &c, &b.supplier, b.amount);
        b.active = false;
        save_p(&env, &DataKey::Bond(bond_id), &b);
        bump(&env);
        env.events().publish((symbol_short!("bond_ok"), b.supplier.clone()), (bond_id, b.amount));
        b
    }

    pub fn slash_supplier_bond(
        env: Env,
        bond_id: u64,
        penalty_bps: u32,
        treasury_recovery: Address,
    ) -> i128 {
        let c = cfg(&env);
        c.admin.require_auth();
        if penalty_bps == 0 || penalty_bps > 10_000 {
            panic_with_error!(&env, StreamError::InvalidBps);
        }
        let mut b: BondRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bond(bond_id))
            .unwrap_or_else(|| panic_with_error!(&env, StreamError::NotFound));
        if !b.active {
            panic_with_error!(&env, StreamError::Inactive);
        }
        let penalty = (b.amount * (penalty_bps as i128)) / 10_000;
        let refund = b.amount - penalty;
        if penalty > 0 {
            pay_out(&env, &c, &treasury_recovery, penalty);
        }
        if refund > 0 {
            pay_out(&env, &c, &b.supplier, refund);
        }
        b.active = false;
        save_p(&env, &DataKey::Bond(bond_id), &b);
        bump(&env);
        env.events().publish((symbol_short!("bond_sl"), b.supplier), (bond_id, penalty));
        penalty
    }

    pub fn pay_retention_warranty_holdback(
        env: Env,
        supplier: Address,
        gross_work: i128,
        holdback_bps: u32,
    ) -> WarrantyHold {
        let c = cfg(&env);
        c.operator.require_auth();
        if gross_work <= 0 || holdback_bps < 100 || holdback_bps > 3_000 {
            panic_with_error!(&env, StreamError::InvalidBps);
        }
        let holdback = (gross_work * (holdback_bps as i128)) / 10_000;
        let immediate = gross_work - holdback;
        pay_out(&env, &c, &supplier, immediate);
        let id = next_id(&env);
        let w = WarrantyHold { id, supplier: supplier.clone(), holdback, active: true };
        save_p(&env, &DataKey::Warranty(id), &w);
        bump(&env);
        env.events().publish((symbol_short!("warr_new"), supplier), (id, immediate, holdback));
        w
    }

    pub fn claim_warranty_holdback(env: Env, warranty_id: u64) -> WarrantyHold {
        let c = cfg(&env);
        c.admin.require_auth();
        let mut w: WarrantyHold = env
            .storage()
            .persistent()
            .get(&DataKey::Warranty(warranty_id))
            .unwrap_or_else(|| panic_with_error!(&env, StreamError::NotFound));
        if !w.active {
            panic_with_error!(&env, StreamError::Inactive);
        }
        pay_out(&env, &c, &w.supplier, w.holdback);
        w.active = false;
        save_p(&env, &DataKey::Warranty(warranty_id), &w);
        bump(&env);
        env.events().publish((symbol_short!("warr_ok"), w.supplier.clone()), (warranty_id, w.holdback));
        w
    }
    // __MODULES_ANCHOR__
}

#[cfg(test)]
mod test;
