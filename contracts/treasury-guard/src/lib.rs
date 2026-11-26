#![no_std]
//! TreasuryGuard — bóveda de tesorería con políticas, sobre Soroban (Stellar).
//!
//! Idea central: la IA de voz NUNCA tiene acceso libre al dinero.
//! El agente (operator) solo puede *solicitar* pagos; el contrato aplica las reglas:
//!   1. Lista blanca: solo proveedores dados de alta por el dueño (admin).
//!   2. Tope por transacción por proveedor.
//!   3. Tope diario global de gasto.
//!   4. Umbral de aprobación: arriba de cierto monto el pago queda "Pending"
//!      hasta que el dueño lo apruebe (segundo factor humano).
//! Los fondos viven en el contrato (token MXNB vía Stellar Asset Contract).

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env, String, Vec,
};

const DAY_SECS: u64 = 86_400;
const INSTANCE_TTL: u32 = 518_400; // ~30 días en ledgers de 5s
const PERSIST_TTL: u32 = 3_110_400; // ~180 días

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub admin: Address,
    pub operator: Address,
    pub token: Address,
    pub daily_limit: i128,
    pub approval_threshold: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Supplier {
    pub name: String,
    pub max_per_tx: i128,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
    Pending = 0,
    Paid = 1,
    Rejected = 2,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Payment {
    pub id: u64,
    pub supplier: Address,
    pub amount: i128,
    pub invoice: String,
    pub status: Status,
    pub created_at: u64,
}

#[contracttype]
pub enum DataKey {
    Config,
    NextId,
    Paused,
    PauseReason,
    Supplier(Address),
    Payment(u64),
    Spent(u64),
    Invoice(String),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotWhitelisted = 1,
    ExceedsTxLimit = 2,
    ExceedsDailyLimit = 3,
    InvalidAmount = 4,
    NotFound = 5,
    NotPending = 6,
    InsufficientFunds = 7,
    DuplicateInvoice = 8,
    Expired = 9,
    VaultPaused = 10,
}

const PENDING_TTL_SECS: u64 = 3 * DAY_SECS; // un pago retenido caduca a los 3 días

#[contract]
pub struct TreasuryGuard;

fn config(env: &Env) -> Config {
    env.storage().instance().get(&DataKey::Config).unwrap()
}

fn bump(env: &Env) {
    env.storage().instance().extend_ttl(INSTANCE_TTL / 2, INSTANCE_TTL);
}

fn today(env: &Env) -> u64 {
    env.ledger().timestamp() / DAY_SECS
}

fn spent(env: &Env) -> i128 {
    env.storage().temporary().get(&DataKey::Spent(today(env))).unwrap_or(0)
}

fn require_not_paused(env: &Env) {
    if env.storage().instance().get(&DataKey::Paused).unwrap_or(false) {
        panic_with_error!(env, Error::VaultPaused);
    }
}

fn save_payment(env: &Env, p: &Payment) {
    let key = DataKey::Payment(p.id);
    env.storage().persistent().set(&key, p);
    env.storage().persistent().extend_ttl(&key, PERSIST_TTL / 2, PERSIST_TTL);
}

/// Mueve los fondos y actualiza el gasto del día. Revalida tope diario y saldo.
fn execute(env: &Env, cfg: &Config, p: &mut Payment) {
    let new_spent = spent(env) + p.amount;
    if new_spent > cfg.daily_limit {
        panic_with_error!(env, Error::ExceedsDailyLimit);
    }
    let tk = token::Client::new(env, &cfg.token);
    let me = env.current_contract_address();
    if tk.balance(&me) < p.amount {
        panic_with_error!(env, Error::InsufficientFunds);
    }
    tk.transfer(&me, &p.supplier, &p.amount);

    let key = DataKey::Spent(today(env));
    env.storage().temporary().set(&key, &new_spent);
    env.storage().temporary().extend_ttl(&key, 17_280 * 2, 17_280 * 2);

    p.status = Status::Paid;
    save_payment(env, p);
    env.events()
        .publish((symbol_short!("paid"), p.supplier.clone()), (p.id, p.amount));
}

#[contractimpl]
impl TreasuryGuard {
    pub fn __constructor(
        env: Env,
        admin: Address,
        operator: Address,
        token: Address,
        daily_limit: i128,
        approval_threshold: i128,
    ) {
        let cfg = Config { admin, operator, token, daily_limit, approval_threshold };
        env.storage().instance().set(&DataKey::Config, &cfg);
        env.storage().instance().set(&DataKey::NextId, &1u64);
        bump(&env);
    }

    // ---------- Administración (solo el dueño) ----------

    pub fn set_supplier(env: Env, supplier: Address, name: String, max_per_tx: i128) {
        let cfg = config(&env);
        cfg.admin.require_auth();
        if max_per_tx <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let key = DataKey::Supplier(supplier.clone());
        env.storage().persistent().set(&key, &Supplier { name, max_per_tx, active: true });
        env.storage().persistent().extend_ttl(&key, PERSIST_TTL / 2, PERSIST_TTL);
        env.events().publish((symbol_short!("supplier"), supplier), max_per_tx);
        bump(&env);
    }

    pub fn remove_supplier(env: Env, supplier: Address) {
        let cfg = config(&env);
        cfg.admin.require_auth();
        let key = DataKey::Supplier(supplier.clone());
        let mut s: Supplier = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotFound));
        s.active = false;
        env.storage().persistent().set(&key, &s);
        bump(&env);
        env.events().publish((symbol_short!("removed"), supplier), ());
    }

    pub fn set_limits(env: Env, daily_limit: i128, approval_threshold: i128) {
        let mut cfg = config(&env);
        cfg.admin.require_auth();
        cfg.daily_limit = daily_limit;
        cfg.approval_threshold = approval_threshold;
        env.storage().instance().set(&DataKey::Config, &cfg);
        bump(&env);
    }

    /// Solo para ensayos: el dueño reinicia el gasto del día (el límite NO cambia).
    pub fn reset_day(env: Env) {
        let cfg = config(&env);
        cfg.admin.require_auth();
        env.storage().temporary().remove(&DataKey::Spent(today(&env)));
        env.events().publish((symbol_short!("reset"),), today(&env));
        bump(&env);
    }

    /// Botón de pánico antifraude: congela todas las salidas de fondos de la bóveda al instante.
    pub fn pause_vault(env: Env, reason: String) {
        let cfg = config(&env);
        cfg.admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &true);
        env.storage().instance().set(&DataKey::PauseReason, &reason);
        bump(&env);
        env.events()
            .publish((symbol_short!("paused"), cfg.admin), (env.ledger().timestamp(), reason));
    }

    /// Reactiva la operación de la bóveda tras auditoría del director.
    pub fn unpause_vault(env: Env) {
        let cfg = config(&env);
        cfg.admin.require_auth();
        let was_paused: bool = env.storage().instance().get(&DataKey::Paused).unwrap_or(false);
        if !was_paused {
            panic_with_error!(&env, Error::NotPending);
        }
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().remove(&DataKey::PauseReason);
        bump(&env);
        env.events()
            .publish((symbol_short!("unpaused"), cfg.admin), env.ledger().timestamp());
    }

    // ---------- Operación (agente de IA) ----------

    /// El agente solicita un pago. Si está bajo el umbral se liquida al instante;
    /// si lo supera queda Pending hasta que el dueño apruebe.
    pub fn request_payment(env: Env, supplier: Address, amount: i128, invoice: String) -> Payment {
        let cfg = config(&env);
        cfg.operator.require_auth();
        require_not_paused(&env);
        bump(&env);

        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let s: Supplier = env
            .storage()
            .persistent()
            .get(&DataKey::Supplier(supplier.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotWhitelisted));
        if !s.active {
            panic_with_error!(&env, Error::NotWhitelisted);
        }
        if amount > s.max_per_tx {
            panic_with_error!(&env, Error::ExceedsTxLimit);
        }

        // Una factura solo se puede pagar (o retener) una vez: candado contra doble pago
        let inv_key = DataKey::Invoice(invoice.clone());
        if env.storage().persistent().has(&inv_key) {
            panic_with_error!(&env, Error::DuplicateInvoice);
        }

        let id: u64 = env.storage().instance().get(&DataKey::NextId).unwrap_or(1);
        env.storage().persistent().set(&inv_key, &id);
        env.storage().persistent().extend_ttl(&inv_key, PERSIST_TTL / 2, PERSIST_TTL);
        env.storage().instance().set(&DataKey::NextId, &(id + 1));
        let mut p = Payment {
            id,
            supplier: supplier.clone(),
            amount,
            invoice,
            status: Status::Pending,
            created_at: env.ledger().timestamp(),
        };

        if amount > cfg.approval_threshold {
            save_payment(&env, &p);
            env.events().publish((symbol_short!("pending"), supplier), (id, amount));
        } else {
            execute(&env, &cfg, &mut p);
        }
        p
    }

    /// Segundo factor humano: el dueño aprueba un pago retenido.
    pub fn approve(env: Env, id: u64) -> Payment {
        let cfg = config(&env);
        cfg.admin.require_auth();
        require_not_paused(&env);
        let mut p: Payment = env
            .storage()
            .persistent()
            .get(&DataKey::Payment(id))
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotFound));
        if p.status != Status::Pending {
            panic_with_error!(&env, Error::NotPending);
        }
        if env.ledger().timestamp() > p.created_at + PENDING_TTL_SECS {
            panic_with_error!(&env, Error::Expired);
        }
        // El proveedor debe seguir autorizado al momento de liberar el pago
        let s: Supplier = env
            .storage()
            .persistent()
            .get(&DataKey::Supplier(p.supplier.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotWhitelisted));
        if !s.active {
            panic_with_error!(&env, Error::NotWhitelisted);
        }
        execute(&env, &cfg, &mut p);
        bump(&env);
        p
    }

    pub fn reject(env: Env, id: u64) -> Payment {
        let cfg = config(&env);
        cfg.admin.require_auth();
        let mut p: Payment = env
            .storage()
            .persistent()
            .get(&DataKey::Payment(id))
            .unwrap_or_else(|| panic_with_error!(&env, Error::NotFound));
        if p.status != Status::Pending {
            panic_with_error!(&env, Error::NotPending);
        }
        p.status = Status::Rejected;
        save_payment(&env, &p);
        env.storage().persistent().remove(&DataKey::Invoice(p.invoice.clone()));
        bump(&env);
        env.events().publish((symbol_short!("rejected"), p.supplier.clone()), id);
        p
    }

    // ---------- Lectura ----------

    pub fn get_config(env: Env) -> Config {
        config(&env)
    }

    pub fn get_supplier(env: Env, supplier: Address) -> Option<Supplier> {
        env.storage().persistent().get(&DataKey::Supplier(supplier))
    }

    pub fn get_payment(env: Env, id: u64) -> Option<Payment> {
        env.storage().persistent().get(&DataKey::Payment(id))
    }

    /// ¿Ya existe un pago (liquidado o retenido) para esta factura? Devuelve su id.
    pub fn invoice_payment(env: Env, invoice: String) -> Option<u64> {
        env.storage().persistent().get(&DataKey::Invoice(invoice))
    }

    pub fn spent_today(env: Env) -> i128 {
        spent(&env)
    }

    pub fn balance(env: Env) -> i128 {
        let cfg = config(&env);
        token::Client::new(&env, &cfg.token).balance(&env.current_contract_address())
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage().instance().get(&DataKey::Paused).unwrap_or(false)
    }
}

#[cfg(test)]
mod test;
