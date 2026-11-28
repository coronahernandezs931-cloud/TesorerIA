#![no_std]
#![allow(deprecated)]
//! TreasurySettlement — compensación SPEI, arrendamiento industrial, certificados de almacén,
//! avío agrícola, adquirencia POS, impuestos locales (Predial/ISN) y caps legales en Soroban.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env,
};

const INSTANCE_TTL: u32 = 518_400;
const PERSIST_TTL: u32 = 3_110_400;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SettleConfig {
    pub admin: Address,
    pub operator: Address,
    pub token: Address,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LeaseContract {
    pub id: u64,
    pub lessor: Address,
    pub monthly_rent: i128,
    pub buyout_price: i128,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct WarrantLoan {
    pub id: u64,
    pub principal: i128,
    pub redeemed: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SyndicatedEscrow {
    pub id: u64,
    pub contractor: Address,
    pub remaining: i128,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct HarvestTicket {
    pub id: u64,
    pub producer: Address,
    pub advance: i128,
    pub settled: bool,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LegalCap {
    pub id: u64,
    pub counsel: Address,
    pub remaining: i128,
    pub active: bool,
}

#[contracttype]
pub enum DataKey {
    Config,
    NextId,
    ClearingPool,
    ChargebackRes,
    Lease(u64),
    Warrant(u64),
    Syndicate(u64),
    Harvest(u64),
    Legal(u64),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SettleError {
    InvalidAmount = 1,
    InvalidBps = 2,
    NotFound = 3,
    Inactive = 4,
    InsufficientFunds = 5,
}

#[contract]
pub struct TreasurySettlement;

fn cfg(env: &Env) -> SettleConfig {
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

fn transfer_out(env: &Env, c: &SettleConfig, to: &Address, amount: i128) {
    if amount <= 0 {
        panic_with_error!(env, SettleError::InvalidAmount);
    }
    let tk = token::Client::new(env, &c.token);
    let me = env.current_contract_address();
    if tk.balance(&me) < amount {
        panic_with_error!(env, SettleError::InsufficientFunds);
    }
    tk.transfer(&me, to, &amount);
}

#[contractimpl]
impl TreasurySettlement {
    pub fn __constructor(env: Env, admin: Address, operator: Address, token: Address) {
        env.storage().instance().set(&DataKey::Config, &SettleConfig { admin, operator, token });
        env.storage().instance().set(&DataKey::NextId, &1u64);
        bump(&env);
    }

    pub fn fund_settlement_clearing(env: Env, funder: Address, amount: i128) -> i128 {
        let c = cfg(&env);
        funder.require_auth();
        if amount <= 0 { panic_with_error!(&env, SettleError::InvalidAmount); }
        token::Client::new(&env, &c.token).transfer(&funder, &env.current_contract_address(), &amount);
        let pool = env.storage().instance().get(&DataKey::ClearingPool).unwrap_or(0i128) + amount;
        env.storage().instance().set(&DataKey::ClearingPool, &pool);
        bump(&env);
        env.events().publish((symbol_short!("clr_fund"), funder), amount);
        pool
    }

    pub fn clear_spei_mirror_payout(env: Env, spei_anchor: Address, fee_desk: Address, gross_mxn: i128, rail_fee: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if gross_mxn <= 0 || rail_fee <= 0 || rail_fee >= gross_mxn { panic_with_error!(&env, SettleError::InvalidAmount); }
        let net_spei = gross_mxn - rail_fee;
        transfer_out(&env, &c, &spei_anchor, net_spei);
        transfer_out(&env, &c, &fee_desk, rail_fee);
        bump(&env);
        net_spei
    }

    pub fn open_equipment_lease(env: Env, lessor: Address, deposit_and_first: i128, monthly_rent: i128, buyout_price: i128) -> LeaseContract {
        let c = cfg(&env);
        c.admin.require_auth();
        if deposit_and_first <= 0 || monthly_rent <= 0 || buyout_price <= 0 { panic_with_error!(&env, SettleError::InvalidAmount); }
        transfer_out(&env, &c, &lessor, deposit_and_first);
        let id = next_id(&env);
        let ls = LeaseContract { id, lessor, monthly_rent, buyout_price, active: true };
        save_p(&env, &DataKey::Lease(id), &ls);
        bump(&env);
        ls
    }

    pub fn pay_monthly_lease_rent(env: Env, lease_id: u64) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        let ls: LeaseContract = env.storage().persistent().get(&DataKey::Lease(lease_id)).unwrap_or_else(|| panic_with_error!(&env, SettleError::NotFound));
        if !ls.active { panic_with_error!(&env, SettleError::Inactive); }
        transfer_out(&env, &c, &ls.lessor, ls.monthly_rent);
        bump(&env);
        ls.monthly_rent
    }

    pub fn exercise_lease_buyout(env: Env, lease_id: u64) -> LeaseContract {
        let c = cfg(&env);
        c.admin.require_auth();
        let mut ls: LeaseContract = env.storage().persistent().get(&DataKey::Lease(lease_id)).unwrap_or_else(|| panic_with_error!(&env, SettleError::NotFound));
        if !ls.active { panic_with_error!(&env, SettleError::Inactive); }
        transfer_out(&env, &c, &ls.lessor, ls.buyout_price);
        ls.active = false;
        save_p(&env, &DataKey::Lease(lease_id), &ls);
        bump(&env);
        ls
    }

    pub fn pledge_warehouse_warrant(env: Env, borrower: Address, warehouse: Address, loan_principal: i128, storage_fee: i128) -> WarrantLoan {
        let c = cfg(&env);
        c.operator.require_auth();
        if loan_principal <= 0 || storage_fee <= 0 || storage_fee >= loan_principal { panic_with_error!(&env, SettleError::InvalidAmount); }
        transfer_out(&env, &c, &borrower, loan_principal - storage_fee);
        transfer_out(&env, &c, &warehouse, storage_fee);
        let id = next_id(&env);
        let wl = WarrantLoan { id, principal: loan_principal, redeemed: false };
        save_p(&env, &DataKey::Warrant(id), &wl);
        bump(&env);
        wl
    }

    pub fn redeem_warehouse_warrant(env: Env, payer: Address, warrant_id: u64) -> WarrantLoan {
        let c = cfg(&env);
        payer.require_auth();
        let mut wl: WarrantLoan = env.storage().persistent().get(&DataKey::Warrant(warrant_id)).unwrap_or_else(|| panic_with_error!(&env, SettleError::NotFound));
        if wl.redeemed { panic_with_error!(&env, SettleError::Inactive); }
        token::Client::new(&env, &c.token).transfer(&payer, &env.current_contract_address(), &wl.principal);
        wl.redeemed = true;
        save_p(&env, &DataKey::Warrant(warrant_id), &wl);
        bump(&env);
        wl
    }

    pub fn settle_consignment_sold(env: Env, consignor: Address, margin_vault: Address, gross_sales: i128, margin_bps: u32) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if gross_sales <= 0 || margin_bps == 0 || margin_bps >= 6_000 { panic_with_error!(&env, SettleError::InvalidBps); }
        let margin = (gross_sales * (margin_bps as i128)) / 10_000;
        let consignor_net = gross_sales - margin;
        transfer_out(&env, &c, &consignor, consignor_net);
        transfer_out(&env, &c, &margin_vault, margin);
        bump(&env);
        consignor_net
    }

    pub fn pay_franchise_fee_split(env: Env, franchisor: Address, ad_fund_vault: Address, monthly_sales: i128, royalty_bps: u32, ad_bps: u32) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if monthly_sales <= 0 || royalty_bps == 0 || ad_bps == 0 || royalty_bps + ad_bps > 2_500 { panic_with_error!(&env, SettleError::InvalidBps); }
        let roy = (monthly_sales * (royalty_bps as i128)) / 10_000;
        let ad = (monthly_sales * (ad_bps as i128)) / 10_000;
        transfer_out(&env, &c, &franchisor, roy);
        transfer_out(&env, &c, &ad_fund_vault, ad);
        bump(&env);
        roy + ad
    }

    pub fn open_syndicated_escrow(env: Env, contractor: Address, arranger: Address, capex_budget: i128, arranger_fee: i128) -> SyndicatedEscrow {
        let c = cfg(&env);
        c.admin.require_auth();
        if capex_budget <= 0 || arranger_fee <= 0 || arranger_fee >= capex_budget { panic_with_error!(&env, SettleError::InvalidAmount); }
        transfer_out(&env, &c, &arranger, arranger_fee);
        let id = next_id(&env);
        let se = SyndicatedEscrow { id, contractor, remaining: capex_budget - arranger_fee, active: true };
        save_p(&env, &DataKey::Syndicate(id), &se);
        bump(&env);
        se
    }

    pub fn release_syndicated_draw(env: Env, escrow_id: u64, draw_amount: i128) -> SyndicatedEscrow {
        let c = cfg(&env);
        c.admin.require_auth();
        let mut se: SyndicatedEscrow = env.storage().persistent().get(&DataKey::Syndicate(escrow_id)).unwrap_or_else(|| panic_with_error!(&env, SettleError::NotFound));
        if !se.active || draw_amount <= 0 || draw_amount > se.remaining { panic_with_error!(&env, SettleError::InvalidAmount); }
        transfer_out(&env, &c, &se.contractor, draw_amount);
        se.remaining -= draw_amount;
        if se.remaining == 0 { se.active = false; }
        save_p(&env, &DataKey::Syndicate(escrow_id), &se);
        bump(&env);
        se
    }

    pub fn pay_notary_fideicomiso_fee(env: Env, notary: Address, public_registry: Address, honorarios: i128, derechos: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if honorarios <= 0 || derechos <= 0 { panic_with_error!(&env, SettleError::InvalidAmount); }
        transfer_out(&env, &c, &notary, honorarios);
        transfer_out(&env, &c, &public_registry, derechos);
        bump(&env);
        honorarios + derechos
    }

    pub fn disburse_harvest_advance(env: Env, producer: Address, avio_advance: i128) -> HarvestTicket {
        let c = cfg(&env);
        c.operator.require_auth();
        transfer_out(&env, &c, &producer, avio_advance);
        let id = next_id(&env);
        let ht = HarvestTicket { id, producer, advance: avio_advance, settled: false };
        save_p(&env, &DataKey::Harvest(id), &ht);
        bump(&env);
        ht
    }

    pub fn liquidate_harvest_delivery(env: Env, ticket_id: u64, gross_crop_value: i128) -> HarvestTicket {
        let c = cfg(&env);
        c.operator.require_auth();
        let mut ht: HarvestTicket = env.storage().persistent().get(&DataKey::Harvest(ticket_id)).unwrap_or_else(|| panic_with_error!(&env, SettleError::NotFound));
        if ht.settled || gross_crop_value <= ht.advance { panic_with_error!(&env, SettleError::InvalidAmount); }
        let net_crop = gross_crop_value - ht.advance;
        transfer_out(&env, &c, &ht.producer, net_crop);
        ht.settled = true;
        save_p(&env, &DataKey::Harvest(ticket_id), &ht);
        bump(&env);
        ht
    }

    pub fn settle_card_acquirer_batch(env: Env, merchant_unit: Address, acquirer_bank: Address, batch_volume: i128, mdr_bps: u32) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if batch_volume <= 0 || mdr_bps == 0 || mdr_bps > 1_000 { panic_with_error!(&env, SettleError::InvalidBps); }
        let mdr = (batch_volume * (mdr_bps as i128)) / 10_000;
        let net = batch_volume - mdr;
        transfer_out(&env, &c, &merchant_unit, net);
        transfer_out(&env, &c, &acquirer_bank, mdr);
        bump(&env);
        net
    }

    pub fn fund_chargeback_reserve(env: Env, sponsor: Address, amount: i128) -> i128 {
        let c = cfg(&env);
        sponsor.require_auth();
        if amount <= 0 { panic_with_error!(&env, SettleError::InvalidAmount); }
        token::Client::new(&env, &c.token).transfer(&sponsor, &env.current_contract_address(), &amount);
        let res = env.storage().instance().get(&DataKey::ChargebackRes).unwrap_or(0i128) + amount;
        env.storage().instance().set(&DataKey::ChargebackRes, &res);
        bump(&env);
        res
    }

    pub fn debit_lost_chargeback(env: Env, acquirer_bank: Address, dispute_amount: i128) -> i128 {
        let c = cfg(&env);
        c.admin.require_auth();
        let res = env.storage().instance().get(&DataKey::ChargebackRes).unwrap_or(0i128);
        if dispute_amount <= 0 || dispute_amount > res { panic_with_error!(&env, SettleError::InsufficientFunds); }
        transfer_out(&env, &c, &acquirer_bank, dispute_amount);
        let next = res - dispute_amount;
        env.storage().instance().set(&DataKey::ChargebackRes, &next);
        bump(&env);
        next
    }

    pub fn pay_municipality_predial(env: Env, tesoreria_municipal: Address, savings_vault: Address, assessed_tax: i128, prompt_discount_bps: u32) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if assessed_tax <= 0 || prompt_discount_bps == 0 || prompt_discount_bps > 2_500 { panic_with_error!(&env, SettleError::InvalidBps); }
        let discount = (assessed_tax * (prompt_discount_bps as i128)) / 10_000;
        let paid = assessed_tax - discount;
        transfer_out(&env, &c, &tesoreria_municipal, paid);
        transfer_out(&env, &c, &savings_vault, discount);
        bump(&env);
        paid
    }

    pub fn remit_state_payroll_isn(env: Env, secretaria_finanzas: Address, state_wage_base: i128, isn_bps: u32) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if state_wage_base <= 0 || isn_bps < 100 || isn_bps > 600 { panic_with_error!(&env, SettleError::InvalidBps); }
        let isn = (state_wage_base * (isn_bps as i128)) / 10_000;
        transfer_out(&env, &c, &secretaria_finanzas, isn);
        bump(&env);
        isn
    }

    pub fn pay_carrier_demurrage(env: Env, port_terminal: Address, shipping_line: Address, days_late: u32, terminal_daily: i128, line_daily: i128) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if days_late == 0 || terminal_daily <= 0 || line_daily <= 0 { panic_with_error!(&env, SettleError::InvalidAmount); }
        let term_amt = (days_late as i128) * terminal_daily;
        let line_amt = (days_late as i128) * line_daily;
        transfer_out(&env, &c, &port_terminal, term_amt);
        transfer_out(&env, &c, &shipping_line, line_amt);
        bump(&env);
        term_amt + line_amt
    }

    pub fn disburse_r_and_d_grant(env: Env, research_team: Address, university_overhead: Address, grant_total: i128, overhead_bps: u32) -> i128 {
        let c = cfg(&env);
        c.admin.require_auth();
        if grant_total <= 0 || overhead_bps == 0 || overhead_bps > 3_000 { panic_with_error!(&env, SettleError::InvalidBps); }
        let ovh = (grant_total * (overhead_bps as i128)) / 10_000;
        let direct = grant_total - ovh;
        transfer_out(&env, &c, &research_team, direct);
        transfer_out(&env, &c, &university_overhead, ovh);
        bump(&env);
        direct
    }

    pub fn settle_saas_annual_prepay(env: Env, saas_vendor: Address, it_rebate_vault: Address, seats: u32, monthly_seat_price: i128, annual_discount_bps: u32) -> i128 {
        let c = cfg(&env);
        c.operator.require_auth();
        if seats == 0 || monthly_seat_price <= 0 || annual_discount_bps == 0 || annual_discount_bps > 3_500 { panic_with_error!(&env, SettleError::InvalidBps); }
        let list_annual = (seats as i128) * monthly_seat_price * 12;
        let savings = (list_annual * (annual_discount_bps as i128)) / 10_000;
        let net_saas = list_annual - savings;
        transfer_out(&env, &c, &saas_vendor, net_saas);
        transfer_out(&env, &c, &it_rebate_vault, savings);
        bump(&env);
        net_saas
    }

    pub fn pay_legal_retainer_cap(env: Env, counsel: Address, initial_retainer: i128, matter_cap: i128) -> LegalCap {
        let c = cfg(&env);
        c.admin.require_auth();
        if initial_retainer <= 0 || matter_cap <= initial_retainer { panic_with_error!(&env, SettleError::InvalidAmount); }
        transfer_out(&env, &c, &counsel, initial_retainer);
        let id = next_id(&env);
        let lc = LegalCap { id, counsel, remaining: matter_cap - initial_retainer, active: true };
        save_p(&env, &DataKey::Legal(id), &lc);
        bump(&env);
        lc
    }

    pub fn draw_litigation_expense(env: Env, legal_id: u64, expense: i128) -> LegalCap {
        let c = cfg(&env);
        c.operator.require_auth();
        let mut lc: LegalCap = env.storage().persistent().get(&DataKey::Legal(legal_id)).unwrap_or_else(|| panic_with_error!(&env, SettleError::NotFound));
        if !lc.active || expense <= 0 || expense > lc.remaining { panic_with_error!(&env, SettleError::InvalidAmount); }
        transfer_out(&env, &c, &lc.counsel, expense);
        lc.remaining -= expense;
        if lc.remaining == 0 { lc.active = false; }
        save_p(&env, &DataKey::Legal(legal_id), &lc);
        bump(&env);
        lc
    }

    pub fn sweep_closed_matter_cap(env: Env, legal_id: u64, treasury_dest: Address) -> i128 {
        let c = cfg(&env);
        c.admin.require_auth();
        let mut lc: LegalCap = env.storage().persistent().get(&DataKey::Legal(legal_id)).unwrap_or_else(|| panic_with_error!(&env, SettleError::NotFound));
        if !lc.active || lc.remaining <= 0 { panic_with_error!(&env, SettleError::Inactive); }
        let rem = lc.remaining;
        transfer_out(&env, &c, &treasury_dest, rem);
        lc.remaining = 0;
        lc.active = false;
        save_p(&env, &DataKey::Legal(legal_id), &lc);
        bump(&env);
        rem
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient};

    #[test]
    fn test_all_25_settlement_ops() {
        let env = Env::default();
        env.mock_all_auths();
        let adm = Address::generate(&env);
        let op = Address::generate(&env);
        let payer = Address::generate(&env);
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let v = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let tok = sac.address();
        let cid = env.register(TreasurySettlement, (adm, op, tok.clone()));
        let sa = StellarAssetClient::new(&env, &tok);
        sa.mint(&cid, &100_000_000_000_000);
        sa.mint(&payer, &50_000_000_000_000);
        let u = 10_000_000i128;
        let cli = TreasurySettlementClient::new(&env, &cid);

        cli.fund_settlement_clearing(&payer, &(50_000 * u));
        cli.clear_spei_mirror_payout(&a, &v, &(20_000 * u), &(100 * u));
        let ls = cli.open_equipment_lease(&a, &(10_000 * u), &(5_000 * u), &(15_000 * u));
        cli.pay_monthly_lease_rent(&ls.id);
        cli.exercise_lease_buyout(&ls.id);
        let wl = cli.pledge_warehouse_warrant(&a, &b, &(20_000 * u), &(500 * u));
        cli.redeem_warehouse_warrant(&payer, &wl.id);
        cli.settle_consignment_sold(&a, &v, &(15_000 * u), &1_500);
        cli.pay_franchise_fee_split(&a, &b, &(50_000 * u), &500, &200);
        let se = cli.open_syndicated_escrow(&a, &b, &(40_000 * u), &(1_000 * u));
        cli.release_syndicated_draw(&se.id, &(20_000 * u));
        cli.pay_notary_fideicomiso_fee(&a, &b, &(5_000 * u), &(2_000 * u));
        let ht = cli.disburse_harvest_advance(&a, &(10_000 * u));
        cli.liquidate_harvest_delivery(&ht.id, &(25_000 * u));
        cli.settle_card_acquirer_batch(&a, &b, &(30_000 * u), &250);
        cli.fund_chargeback_reserve(&payer, &(10_000 * u));
        cli.debit_lost_chargeback(&b, &(3_000 * u));
        cli.pay_municipality_predial(&a, &v, &(20_000 * u), &1_000);
        cli.remit_state_payroll_isn(&a, &(100_000 * u), &300);
        cli.pay_carrier_demurrage(&a, &b, &2, &(1_000 * u), &(1_500 * u));
        cli.disburse_r_and_d_grant(&a, &b, &(25_000 * u), &1_000);
        cli.settle_saas_annual_prepay(&a, &v, &10, &(100 * u), &1_000);
        let lc = cli.pay_legal_retainer_cap(&a, &(5_000 * u), &(20_000 * u));
        cli.draw_litigation_expense(&lc.id, &(4_000 * u));
        cli.sweep_closed_matter_cap(&lc.id, &v);
    }
}
