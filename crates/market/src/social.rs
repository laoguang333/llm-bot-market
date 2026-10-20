//! Actual settled experience, not hypothetical demand times price change.
//! All stock and cash values come from the same observation the member sees.
use bot_api::*;

pub(crate) const MEMORY: f64 = 100.0;

pub(crate) struct Tracker {
    pub group: usize,
    initial_equity: f64,
    previous_equity: Money,
    shares: Vec<Qty>,
    marks: Vec<Price>,
    last_tick: Tick,
    profits: Vec<f64>,
    net: f64,
    published: Vec<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn obs(tick: Tick, shares: Qty, price: Price, equity: Money) -> Observation {
        Observation {
            tick,
            ticks_total: 1000,
            reference_tick: tick,
            season: Season {
                turbulent: false,
                rate_mult: 1.0,
                size_mult: 1.0,
            },
            account: AccountView {
                shares: vec![shares],
                equity,
                ..Default::default()
            },
            open_orders: vec![],
            books: vec![BookView {
                mark: price,
                last: price,
                ..Default::default()
            }],
            trades: vec![vec![]],
            skipped: 0,
            fills: vec![],
            acks: vec![],
            rejects: vec![],
            news: vec![],
            season_changes: vec![],
            income: vec![],
            terminal_hints: vec![],
            social: None,
        }
    }
    fn fill(side: Side, price: Price, qty: Qty, fee: Money) -> Fill {
        Fill {
            order_id: 1,
            stock: 0,
            side,
            price,
            qty,
            fee,
            maker: false,
            tick: 1,
        }
    }
    #[test]
    fn actual_partial_fill_then_exit_counts_fees_once_and_keeps_closed_profit() {
        let mut t = Tracker::new(0, 1000, &[0], &[100]);
        let mut o = obs(1, 5, 100, 998);
        o.fills.push(fill(Side::Buy, 100, 5, 2));
        t.update(&o);
        assert_eq!(t.profits[0], -2.0);
        assert_eq!(t.net, -2.0);
        let mut o = obs(2, 0, 110, 1046);
        o.fills.push(fill(Side::Sell, 110, 5, 2));
        t.update(&o);
        let expected = 48.0 - 2.0 * (-0.01_f64).exp();
        assert!((t.profits[0] - expected).abs() < 1e-9);
        t.update(&obs(102, 0, 200, 1046)); // no phantom gains after exit
        assert!((t.profits[0] - expected * (-1.0_f64).exp()).abs() < 1e-9);
    }
    #[test]
    fn dividends_are_stock_profit_burn_and_cash_fee_only_reduce_account_net() {
        let mut t = Tracker::new(0, 1000, &[10], &[100]);
        let mut o = obs(1, 9, 100, 1895);
        o.income = vec![
            Income::Dividend {
                stock: 0,
                amount: 5,
            },
            Income::IdleFee {
                paid: 110,
                gross: 110,
                credit: 0,
            },
        ];
        t.update(&o);
        assert_eq!(t.profits[0], 5.0);
        assert_eq!(t.net, -105.0);
        let mut o = obs(2, 9, 100, 1945);
        o.income = vec![Income::BadDebtWrittenOff { amount: 50 }];
        t.update(&o);
        assert!((t.net - (-105.0) * (-0.01_f64).exp()).abs() < 1e-9);
    }
    #[test]
    fn group_excludes_own_experience_and_unrelated_group() {
        let mut g = Group::new(1);
        g.members = 2;
        let mut a = Tracker::new(0, 1000, &[0], &[100]);
        let mut b = Tracker::new(0, 2000, &[0], &[100]);
        a.profits[0] = 100.0;
        b.profits[0] = -100.0;
        assert_eq!(g.publish(&mut a).peer_stock, vec![0.0]);
        assert!((g.publish(&mut b).peer_stock[0] - 0.1).abs() < 1e-9);
        assert!((g.publish(&mut a).peer_stock[0] + 0.05).abs() < 1e-9);
        let mut unrelated = Group::new(1);
        unrelated.members = 1;
        let mut c = Tracker::new(1, 1000, &[0], &[100]);
        c.profits[0] = 10000.0;
        assert_eq!(unrelated.publish(&mut c).peer_stock, vec![0.0]);
        assert!((g.publish(&mut a).peer_stock[0] + 0.05).abs() < 1e-9);
    }
    #[test]
    fn new_losses_overcome_decaying_old_gains() {
        let mut t = Tracker::new(0, 0, &[10], &[100]);
        t.update(&obs(1, 10, 150, 1500));
        assert!(t.profits[0] > 0.0);
        t.update(&obs(201, 10, 140, 1400));
        assert!(t.profits[0] < 0.0);
    }
}

pub(crate) struct Group {
    pub members: usize,
    sum: Vec<f64>,
}

impl Group {
    pub fn new(stocks: usize) -> Self {
        Self {
            members: 0,
            sum: vec![0.0; stocks],
        }
    }

    pub fn publish(&mut self, member: &mut Tracker) -> SocialView {
        let peers = self.members.saturating_sub(1);
        let mut own = Vec::with_capacity(self.sum.len());
        let mut others = Vec::with_capacity(self.sum.len());
        for s in 0..self.sum.len() {
            let value = member.profits[s] / member.initial_equity;
            self.sum[s] += value - member.published[s];
            member.published[s] = value;
            own.push(value);
            others.push(if peers == 0 {
                0.0
            } else {
                (self.sum[s] - value) / peers as f64
            });
        }
        SocialView {
            memory_ticks: MEMORY,
            peers,
            own_stock: own,
            peer_stock: others,
            own_net: member.net / member.initial_equity,
        }
    }
}

impl Tracker {
    pub fn new(group: usize, cash: Money, shares: &[Qty], marks: &[Price]) -> Self {
        let equity = cash + shares.iter().zip(marks).map(|(q, p)| q * p).sum::<Money>();
        Self {
            group,
            initial_equity: equity.max(1) as f64,
            previous_equity: equity,
            shares: shares.to_vec(),
            marks: marks.to_vec(),
            last_tick: 0,
            profits: vec![0.0; shares.len()],
            net: 0.0,
            published: vec![0.0; shares.len()],
        }
    }

    pub fn update(&mut self, o: &Observation) {
        let decay = (-(o.tick.saturating_sub(self.last_tick) as f64) / MEMORY).exp();
        // Shares purchased/sold and their ACTUAL cash flows since our last observation.
        let mut traded = vec![0; self.shares.len()];
        let mut cashflow = vec![0; self.shares.len()];
        for f in &o.fills {
            let signed = if f.side == Side::Buy { f.qty } else { -f.qty };
            traded[f.stock] += signed;
            cashflow[f.stock] -= signed * f.price + f.fee;
        }
        let mut relief = 0;
        for income in &o.income {
            match *income {
                Income::Dividend { stock, amount } => cashflow[stock] += amount,
                Income::BadDebtWrittenOff { amount } => relief += amount,
                _ => {}
            }
        }
        for s in 0..self.shares.len() {
            let q = o.account.shares[s];
            let p = o.books[s].mark;
            // Idle fees can destroy shares without a fill. Remove that transfer
            // from stock trading experience; it remains a whole-account cost.
            let transfer = q - self.shares[s] - traded[s];
            let profit = q * p - self.shares[s] * self.marks[s] + cashflow[s] - transfer * p;
            self.profits[s] = self.profits[s] * decay + profit as f64;
            self.shares[s] = q;
            self.marks[s] = p;
        }
        self.net = self.net * decay + (o.account.equity - self.previous_equity - relief) as f64;
        self.previous_equity = o.account.equity;
        self.last_tick = o.tick;
    }
}
