//! Conserved opening endowments. Never runs during trading or reads world data.

use crate::account::Account;
use crate::config::{Config, MAX_CASH, MAX_SHARES};
use anyhow::{bail, Result};
use std::collections::BTreeMap;

pub(crate) fn validate(cfg: &Config) -> Result<()> {
    let Some(plan) = &cfg.capital_redistribution else {
        return Ok(());
    };
    if !(0..=MAX_CASH).contains(&plan.cash_each)
        || !(0..=MAX_SHARES).contains(&plan.shares_each)
        || (plan.cash_each == 0 && plan.shares_each == 0)
        || plan.recipients.is_empty()
        || plan.recipients.contains_key(&plan.from_bot)
        || plan
            .recipients
            .values()
            .any(|w| !(1..=1_000_000).contains(w))
    {
        bail!("capital_redistribution: invalid transfer amounts or recipient weights");
    }
    let donors: Vec<_> = cfg
        .bots
        .iter()
        .filter(|g| g.owner.is_none() && g.bot == plan.from_bot)
        .collect();
    if !donors.iter().any(|g| g.count > 0) {
        bail!("capital_redistribution: source needs a positive anonymous minimum population");
    }
    for g in donors {
        if g.cash < plan.cash_each
            || cfg
                .stocks
                .iter()
                .any(|s| *g.shares.get(&s.name).unwrap_or(&g.shares_each) < plan.shares_each)
        {
            bail!("capital_redistribution: source cannot fund the transfer");
        }
    }
    for name in plan.recipients.keys() {
        if !cfg
            .bots
            .iter()
            .any(|g| g.owner.is_none() && g.bot == *name && g.count > 0)
        {
            bail!("capital_redistribution: recipient {name} needs a positive anonymous minimum population");
        }
    }
    Ok(())
}

pub(crate) fn redistribute(cfg: &Config, counts: &[usize], accounts: &mut [Account]) -> Result<()> {
    let Some(plan) = &cfg.capital_redistribution else {
        return Ok(());
    };
    let mut eligible = BTreeMap::<&str, Vec<usize>>::new();
    let mut offset = 0;
    for (g, &count) in cfg.bots.iter().zip(counts) {
        if g.owner.is_none() {
            eligible
                .entry(&g.bot)
                .or_default()
                .extend(offset..offset + count);
        }
        offset += count;
    }
    let donors = &eligible[plan.from_bot.as_str()];
    let cash_pool = plan.cash_each as i128 * donors.len() as i128;
    let stock_pool = plan.shares_each as i128 * donors.len() as i128;
    for &i in donors {
        accounts[i].cash -= plan.cash_each;
        for q in &mut accounts[i].shares {
            *q -= plan.shares_each;
        }
    }
    let weight_sum: i128 = plan.recipients.values().map(|w| *w as i128).sum();
    let mut cumulative = 0i128;
    for (name, &weight) in &plan.recipients {
        // Cumulative integer apportionment preserves every cent and share.
        // BTreeMap order and account order deterministically assign remainders.
        let next = cumulative + weight as i128;
        let cash = cash_pool * next / weight_sum - cash_pool * cumulative / weight_sum;
        let stock = stock_pool * next / weight_sum - stock_pool * cumulative / weight_sum;
        let recipients = &eligible[name.as_str()];
        let n = recipients.len() as i128;
        for (j, &i) in recipients.iter().enumerate() {
            let j = j as i128;
            let dc = cash * (j + 1) / n - cash * j / n;
            let dq = stock * (j + 1) / n - stock * j / n;
            let a = &mut accounts[i];
            if a.cash as i128 + dc > MAX_CASH as i128
                || a.shares
                    .iter()
                    .any(|q| *q as i128 + dq > MAX_SHARES as i128)
            {
                bail!(
                    "capital_redistribution: recipient {} exceeds account limits",
                    a.name
                );
            }
            a.cash += dc as i64;
            for q in &mut a.shares {
                *q += dq as i64;
            }
        }
        cumulative = next;
    }
    // Reports and return denominators must use the actual redistributed start.
    for a in accounts {
        a.initial_cash = a.cash;
        a.initial_shares.clone_from(&a.shares);
    }
    Ok(())
}
