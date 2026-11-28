#![no_std]
#![allow(deprecated)]
//! TreasuryTrade — comercio exterior, maquila IMMEX, parques industriales FIBRA,
//! energía limpia PPA/CEL, licitaciones públicas, ferrocarril intermodal, CONAGUA y CAAAREM.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, token,
    Address, Env,
};

const INSTANCE_TTL: u32 = 518_400;
const PERSIST_TTL: u32 = 3_110_400;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TradeConfig {
    pub admin: Address,
    pub operator: Address,
    pub token: Address,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Ticket {
    pub id: u64,
    pub party: Address,
    pub amount: i128,
    pub closed: bool,
}

#[contracttype]
pub enum DataKey {
    Config,
    NextId,
    TradePool,
    RebatePool,
    Export(u64),
    Immex(u64),
    Ppa(u64),
    Tender(u64),
    Coop(u64),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TradeError {
    InvalidAmount = 1,
    InvalidBps = 2,
    NotFound = 3,
    Closed = 4,
    InsufficientFunds = 5,
}

#[contract]
pub struct TreasuryTrade;

fn cfg(env: &Env) -> TradeConfig {
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

fn save_p<K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>, V: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(env: &Env, key: &K, val: &V) {
    env.storage().persistent().set(key, val);
    env.storage().persistent().extend_ttl(key, PERSIST_TTL / 2, PERSIST_TTL);
}

fn send_out(env: &Env, c: &TradeConfig, to: &Address, amount: i128) {
    if amount <= 0 { panic_with_error!(env, TradeError::InvalidAmount); }
    let tk = token::Client::new(env, &c.token);
    let me = env.current_contract_address();
    if tk.balance(&me) < amount { panic_with_error!(env, TradeError::InsufficientFunds); }
    tk.transfer(&me, to, &amount);
}

#[contractimpl]
impl TreasuryTrade {
    pub fn __constructor(env: Env, admin: Address, operator: Address, token: Address) {
        env.storage().instance().set(&DataKey::Config, &TradeConfig { admin, operator, token });
        env.storage().instance().set(&DataKey::NextId, &1u64);
        bump(&env);
    }

    pub fn fund_export_trade_desk(env: Env, funder: Address, amount: i128) -> i128 {
        let c = cfg(&env);
        funder.require_auth();
        if amount <= 0 { panic_with_error!(&env, TradeError::InvalidAmount); }
        token::Client::new(&env, &c.token).transfer(&funder, &env.current_contract_address(), &amount);
        let p = env.storage().instance().get(&DataKey::TradePool).unwrap_or(0i128) + amount;
        env.storage().instance().set(&DataKey::TradePool, &p);
        bump(&env);
        p
    }

    pub fn advance_export_receivable(env: Env, exporter: Address, desk_fee_vault: Address, invoice_amt: i128, fee_bps: u32) -> Ticket {
        let c = cfg(&env);
        c.operator.require_auth();
        if invoice_amt <= 0 || fee_bps == 0 || fee_bps > 2_000 { panic_with_error!(&env, TradeError::InvalidBps); }
        let fee = (invoice_amt * (fee_bps as i128)) / 10_000;
        send_out(&env, &c, &exporter, invoice_amt - fee);
        send_out(&env, &c, &desk_fee_vault, fee);
        let id = next_id(&env);
        let t = Ticket { id, party: exporter, amount: invoice_amt, closed: false };
        save_p(&env, &DataKey::Export(id), &t);
        bump(&env);
        t
    }

    pub fn settle_export_collection(env: Env, buyer_payer: Address, export_id: u64) -> Ticket {
        let c = cfg(&env);
        buyer_payer.require_auth();
        let mut t: Ticket = env.storage().persistent().get(&DataKey::Export(export_id)).unwrap_or_else(|| panic_with_error!(&env, TradeError::NotFound));
        if t.closed { panic_with_error!(&env, TradeError::Closed); }
        token::Client::new(&env, &c.token).transfer(&buyer_payer, &env.current_contract_address(), &t.amount);
        t.closed = true;
        save_p(&env, &DataKey::Export(export_id), &t);
        bump(&env);
        t
    }

    pub fn open_immex_maquila_batch(env: Env, maquila_plant: Address, customs_agent: Address, rebate_locked: i128, agent_fee: i128) -> Ticket {
        let c = cfg(&env);
        c.operator.require_auth();
        if rebate_locked <= 0 || agent_fee <= 0 { panic_with_error!(&env, TradeError::InvalidAmount); }
        send_out(&env, &c, &customs_agent, agent_fee);
        let id = next_id(&env);
        let t = Ticket { id, party: maquila_plant, amount: rebate_locked, closed: false };
        save_p(&env, &DataKey::Immex(id), &t);
        bump(&env);
        t
    }

    pub fn close_immex_return_export(env: Env, immex_id: u64) -> Ticket {
        let c = cfg(&env);
        c.operator.require_auth();
        let mut t: Ticket = env.storage().persistent().get(&DataKey::Immex(immex_id)).unwrap_or_else(|| panic_with_error!(&env, TradeError::NotFound));
        if t.closed { panic_with_error!(&env, TradeError::Closed); }
        send_out(&env, &c, &t.party, t.amount);
        t.closed = true;
        save_p(&env, &DataKey::Immex(immex_id), &t);
        bump(&env);
        t
    }

    pub fn pay_fibra_warehouse_rent(env: Env, fibra_trust: Address, park_cam_vault: Address, base_rent: i128, cam_bps: u32) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if base_rent <= 0 || cam_bps == 0 || cam_bps > 3_000 { panic_with_error!(&env, TradeError::InvalidBps); }
        let cam = (base_rent * (cam_bps as i128)) / 10_000;
        send_out(&env, &c, &fibra_trust, base_rent);
        send_out(&env, &c, &park_cam_vault, cam);
        bump(&env);
        base_rent + cam
    }

    pub fn escrow_ppa_solar_energy(env: Env, ipp_generator: Address, grid_cenace: Address, energy_escrow: i128, grid_fee: i128) -> Ticket {
        let c = cfg(&env);
        c.operator.require_auth();
        send_out(&env, &c, &grid_cenace, grid_fee);
        let id = next_id(&env);
        let t = Ticket { id, party: ipp_generator, amount: energy_escrow, closed: false };
        save_p(&env, &DataKey::Ppa(id), &t);
        bump(&env);
        t
    }

    pub fn settle_ppa_clean_cel(env: Env, ppa_id: u64, cel_bonus: i128) -> Ticket {
        let c = cfg(&env);
        c.operator.require_auth();
        let mut t: Ticket = env.storage().persistent().get(&DataKey::Ppa(ppa_id)).unwrap_or_else(|| panic_with_error!(&env, TradeError::NotFound));
        if t.closed || cel_bonus < 0 { panic_with_error!(&env, TradeError::Closed); }
        send_out(&env, &c, &t.party, t.amount + cel_bonus);
        t.closed = true;
        save_p(&env, &DataKey::Ppa(ppa_id), &t);
        bump(&env);
        t
    }

    pub fn post_public_tender_bid(env: Env, bidder: Address, guarantee_amt: i128) -> Ticket {
        let c = cfg(&env);
        bidder.require_auth();
        if guarantee_amt <= 0 { panic_with_error!(&env, TradeError::InvalidAmount); }
        token::Client::new(&env, &c.token).transfer(&bidder, &env.current_contract_address(), &guarantee_amt);
        let id = next_id(&env);
        let t = Ticket { id, party: bidder, amount: guarantee_amt, closed: false };
        save_p(&env, &DataKey::Tender(id), &t);
        bump(&env);
        t
    }

    pub fn refund_unwon_tender_bid(env: Env, tender_id: u64) -> Ticket {
        let c = cfg(&env);
        c.admin.require_auth();
        let mut t: Ticket = env.storage().persistent().get(&DataKey::Tender(tender_id)).unwrap_or_else(|| panic_with_error!(&env, TradeError::NotFound));
        if t.closed { panic_with_error!(&env, TradeError::Closed); }
        send_out(&env, &c, &t.party, t.amount);
        t.closed = true;
        save_p(&env, &DataKey::Tender(tender_id), &t);
        bump(&env);
        t
    }

    pub fn convert_won_bid_to_capex(env: Env, tender_id: u64, project_unit: Address) -> Ticket {
        let c = cfg(&env);
        c.admin.require_auth();
        let mut t: Ticket = env.storage().persistent().get(&DataKey::Tender(tender_id)).unwrap_or_else(|| panic_with_error!(&env, TradeError::NotFound));
        if t.closed { panic_with_error!(&env, TradeError::Closed); }
        send_out(&env, &c, &project_unit, t.amount);
        t.closed = true;
        save_p(&env, &DataKey::Tender(tender_id), &t);
        bump(&env);
        t
    }

    pub fn pay_rail_intermodal_haul(env: Env, rail_carrier: Address, drayage_fleet: Address, rail_leg: i128, drayage_leg: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        send_out(&env, &c, &rail_carrier, rail_leg);
        send_out(&env, &c, &drayage_fleet, drayage_leg);
        bump(&env);
        rail_leg + drayage_leg
    }

    pub fn settle_cold_chain_reefer(env: Env, reefer_carrier: Address, bonus_vault: Address, base_freight: i128, sla_bonus: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        send_out(&env, &c, &reefer_carrier, base_freight);
        send_out(&env, &c, &bonus_vault, sla_bonus);
        bump(&env);
        base_freight + sla_bonus
    }

    pub fn fund_vendor_rebate_tier(env: Env, sponsor: Address, amount: i128) -> i128 {
        let c = cfg(&env);
        sponsor.require_auth();
        if amount <= 0 { panic_with_error!(&env, TradeError::InvalidAmount); }
        token::Client::new(&env, &c.token).transfer(&sponsor, &env.current_contract_address(), &amount);
        let p = env.storage().instance().get(&DataKey::RebatePool).unwrap_or(0i128) + amount;
        env.storage().instance().set(&DataKey::RebatePool, &p);
        bump(&env);
        p
    }

    pub fn claim_annual_volume_rebate(env: Env, distributor: Address, rebate_amt: i128) -> i128 {
        let c = cfg(&env);
        c.admin.require_auth();
        let p = env.storage().instance().get(&DataKey::RebatePool).unwrap_or(0i128);
        if rebate_amt <= 0 || rebate_amt > p { panic_with_error!(&env, TradeError::InsufficientFunds); }
        send_out(&env, &c, &distributor, rebate_amt);
        let rem = p - rebate_amt;
        env.storage().instance().set(&DataKey::RebatePool, &rem);
        bump(&env);
        rem
    }

    pub fn pay_cold_storage_pallet(env: Env, cold_warehouse: Address, insurer: Address, pallets: u32, rate_per_pallet: i128, ins_fee: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if pallets == 0 || rate_per_pallet <= 0 { panic_with_error!(&env, TradeError::InvalidAmount); }
        let stg = (pallets as i128) * rate_per_pallet;
        send_out(&env, &c, &cold_warehouse, stg);
        send_out(&env, &c, &insurer, ins_fee);
        bump(&env);
        stg + ins_fee
    }

    pub fn disburse_coop_caja_loan(env: Env, worker: Address, principal: i128) -> Ticket {
        let c = cfg(&env);
        c.operator.require_auth();
        send_out(&env, &c, &worker, principal);
        let id = next_id(&env);
        let t = Ticket { id, party: worker, amount: principal, closed: false };
        save_p(&env, &DataKey::Coop(id), &t);
        bump(&env);
        t
    }

    pub fn collect_coop_payroll_ded(env: Env, payer: Address, coop_id: u64) -> Ticket {
        let c = cfg(&env);
        payer.require_auth();
        let mut t: Ticket = env.storage().persistent().get(&DataKey::Coop(coop_id)).unwrap_or_else(|| panic_with_error!(&env, TradeError::NotFound));
        if t.closed { panic_with_error!(&env, TradeError::Closed); }
        token::Client::new(&env, &c.token).transfer(&payer, &env.current_contract_address(), &t.amount);
        t.closed = true;
        save_p(&env, &DataKey::Coop(coop_id), &t);
        bump(&env);
        t
    }

    pub fn pay_patent_pct_filing(env: Env, impi_office: Address, ip_counsel: Address, official_fee: i128, legal_fee: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        send_out(&env, &c, &impi_office, official_fee);
        send_out(&env, &c, &ip_counsel, legal_fee);
        bump(&env);
        official_fee + legal_fee
    }

    pub fn settle_iso_audit_cert(env: Env, cert_body: Address, test_lab: Address, audit_fee: i128, lab_fee: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        send_out(&env, &c, &cert_body, audit_fee);
        send_out(&env, &c, &test_lab, lab_fee);
        bump(&env);
        audit_fee + lab_fee
    }

    pub fn pay_water_conagua_right(env: Env, conagua_vault: Address, cubic_meters: u32, tariff_per_m3: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if cubic_meters == 0 || tariff_per_m3 <= 0 { panic_with_error!(&env, TradeError::InvalidAmount); }
        let total = (cubic_meters as i128) * tariff_per_m3;
        send_out(&env, &c, &conagua_vault, total);
        bump(&env);
        total
    }

    pub fn pay_env_profepa_remedy(env: Env, env_contractor: Address, green_fund: Address, work_cost: i128, fund_contrib: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        send_out(&env, &c, &env_contractor, work_cost);
        send_out(&env, &c, &green_fund, fund_contrib);
        bump(&env);
        work_cost + fund_contrib
    }

    pub fn settle_packaging_return(env: Env, supplier: Address, returned_totes: u32, deposit_per_tote: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if returned_totes == 0 || deposit_per_tote <= 0 { panic_with_error!(&env, TradeError::InvalidAmount); }
        let refund = (returned_totes as i128) * deposit_per_tote;
        send_out(&env, &c, &supplier, refund);
        bump(&env);
        refund
    }

    pub fn pay_customs_prevalidacion(env: Env, caaarem_vault: Address, cnt_vault: Address, pedimentos_n: u32, prev_fee: i128, cnt_fee: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if pedimentos_n == 0 || prev_fee <= 0 || cnt_fee <= 0 { panic_with_error!(&env, TradeError::InvalidAmount); }
        let p_tot = (pedimentos_n as i128) * prev_fee;
        let c_tot = (pedimentos_n as i128) * cnt_fee;
        send_out(&env, &c, &caaarem_vault, p_tot);
        send_out(&env, &c, &cnt_vault, c_tot);
        bump(&env);
        p_tot + c_tot
    }

    pub fn sweep_trade_desk_surplus(env: Env, treasury_dest: Address, amount: i128) -> i128 {
        let c = cfg(&env);
        c.admin.require_auth();
        send_out(&env, &c, &treasury_dest, amount);
        bump(&env);
        amount
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient};

    #[test]
    fn test_all_25_trade_ops() {
        let env = Env::default();
        env.mock_all_auths();
        let adm = Address::generate(&env);
        let op = Address::generate(&env);
        let payer = Address::generate(&env);
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let tok = sac.address();
        let cid = env.register(TreasuryTrade, (adm, op, tok.clone()));
        let sa = StellarAssetClient::new(&env, &tok);
        sa.mint(&cid, &100_000_000_000_000);
        sa.mint(&payer, &50_000_000_000_000);
        let u = 10_000_000i128;
        let cli = TreasuryTradeClient::new(&env, &cid);

        cli.fund_export_trade_desk(&payer, &(50_000 * u));
        let ex = cli.advance_export_receivable(&a, &b, &(30_000 * u), &200);
        cli.settle_export_collection(&payer, &ex.id);
        let im = cli.open_immex_maquila_batch(&a, &b, &(20_000 * u), &(1_000 * u));
        cli.close_immex_return_export(&im.id);
        cli.pay_fibra_warehouse_rent(&a, &b, &(40_000 * u), &1_000);
        let pp = cli.escrow_ppa_solar_energy(&a, &b, &(25_000 * u), &(1_500 * u));
        cli.settle_ppa_clean_cel(&pp.id, &(2_000 * u));
        let td1 = cli.post_public_tender_bid(&payer, &(10_000 * u));
        cli.refund_unwon_tender_bid(&td1.id);
        let td2 = cli.post_public_tender_bid(&payer, &(15_000 * u));
        cli.convert_won_bid_to_capex(&td2.id, &a);
        cli.pay_rail_intermodal_haul(&a, &b, &(12_000 * u), &(4_000 * u));
        cli.settle_cold_chain_reefer(&a, &b, &(18_000 * u), &(1_500 * u));
        cli.fund_vendor_rebate_tier(&payer, &(20_000 * u));
        cli.claim_annual_volume_rebate(&a, &(8_000 * u));
        cli.pay_cold_storage_pallet(&a, &b, &20, &(250 * u), &(400 * u));
        let cp = cli.disburse_coop_caja_loan(&a, &(6_000 * u));
        cli.collect_coop_payroll_ded(&payer, &cp.id);
        cli.pay_patent_pct_filing(&a, &b, &(5_000 * u), &(3_000 * u));
        cli.settle_iso_audit_cert(&a, &b, &(8_000 * u), &(2_500 * u));
        cli.pay_water_conagua_right(&a, &500, &(15 * u));
        cli.pay_env_profepa_remedy(&a, &b, &(9_000 * u), &(1_500 * u));
        cli.settle_packaging_return(&a, &40, &(80 * u));
        cli.pay_customs_prevalidacion(&a, &b, &10, &(260 * u), &(90 * u));
        cli.sweep_trade_desk_surplus(&b, &(5_000 * u));
    }
}
