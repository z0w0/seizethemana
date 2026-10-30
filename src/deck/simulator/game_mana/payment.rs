//! Exact matching between cost symbols and eligible produced mana.

use super::{COLORED, COLORLESS, Cost, ManaPool, SpendRestriction};

/// Life loss is minimized before the number of new mana activations.
type SourceCost = (i128, i128);

/// Alternative mana and cast-tap choices sharing one physical resource slot.
#[derive(Clone, Copy)]
struct SourcePrices<'a> {
    mana: &'a [[Option<u32>; 6]],
    virtual_taps: &'a [Option<u8>],
}

/// One actual mana activation or virtual cast payment chosen for a resource.
struct SourceChoice {
    cost: SourceCost,
    mana_type: u8,
    activate: bool,
}

#[cfg(test)]
std::thread_local! {
    /// Count matching work on the current test thread without timing assertions.
    static MATCHER_CALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A complete mana payment prepared against an unchanged pool.
pub(in crate::deck::simulator) struct ManaPayment {
    before: ManaPool,
    after: ManaPool,
    source_choices: Vec<(usize, u8)>,
    source_cost: SourceCost,
}

impl ManaPayment {
    /// Commit only while every mana resource still matches preparation.
    pub(in crate::deck::simulator) fn commit(self, pool: &mut ManaPool) -> bool {
        if *pool != self.before {
            return false;
        }
        *pool = self.after;
        true
    }
}

/// A pool location from which an exact amount can be spent.
#[derive(Clone, Copy)]
enum Origin {
    Fixed(usize),
    NoGeneric(usize),
    Flexible,
    Colorless,
    Credit,
    Treasure,
    Lot(usize),
    Virtual(usize),
}

/// One independent supply of mana with its payment rules.
struct Supply {
    amount: u32,
    types: u8,
    generic: bool,
    origin: Origin,
    one_color: bool,
}

/// One group of equal symbols in the cost.
struct Demand {
    amount: u32,
    types: u8,
}

/// Prepare the complete mana payment without changing the pool.
/// Phyrexian life is charged separately by callers. Snow is unsupported
/// until mana production tracks snow provenance, so snow costs are rejected.
pub(in crate::deck::simulator) fn payment_plan(
    cost: &Cost,
    pool: &ManaPool,
    classes: &[SpendRestriction],
) -> Option<ManaPayment> {
    payment_plan_with_virtual_sources(cost, pool, classes, &[]).map(|(payment, _)| payment)
}

/// Prepare a mana payment that can also use untapped cast resources.
///
/// Each source pays one generic symbol or one allowed colored symbol.
/// Mask bits 0 through 4 allow white, blue, black, red, and green, in order.
/// A zero mask pays only generic mana. Higher bits are ignored.
/// Returned indexes select entries in `virtual_sources`. [`ManaPayment::commit`]
/// changes only the pool; the caller must validate and commit the selected
/// resources together with that pool payment.
pub(in crate::deck::simulator) fn payment_plan_with_virtual_sources(
    cost: &Cost,
    pool: &ManaPool,
    classes: &[SpendRestriction],
    virtual_sources: &[u8],
) -> Option<(ManaPayment, Vec<usize>)> {
    let mut supplies = supplies(pool, classes)?;
    supplies.splice(
        0..0,
        virtual_sources
            .iter()
            .enumerate()
            .map(|(index, types)| Supply {
                amount: 1,
                types: *types & COLORED,
                generic: true,
                origin: Origin::Virtual(index),
                one_color: false,
            }),
    );
    prepare_payment(cost, pool, &mut supplies)
}

/// Match one-unit physical sources, retaining each selected WUBRG or colorless type.
///
/// Bits 0 through 5 are WUBRG and colorless. Zero masks cannot produce mana.
/// Existing pool mana is used first. The returned payment changes only that pool;
/// callers must activate the selected physical sources through their typed abilities.
#[cfg(test)]
pub(in crate::deck::simulator) fn payment_plan_with_source_choices(
    cost: &Cost,
    pool: &ManaPool,
    classes: &[SpendRestriction],
    source_choices: &[u8],
) -> Option<(ManaPayment, Vec<(usize, u8)>)> {
    let mut supplies = supplies(pool, classes)?;
    supplies.extend(
        source_choices
            .iter()
            .enumerate()
            .filter(|(_, types)| **types & (COLORED | COLORLESS) != 0)
            .map(|(index, types)| Supply {
                amount: 1,
                types: *types & (COLORED | COLORLESS),
                generic: true,
                origin: Origin::Virtual(index),
                one_color: false,
            }),
    );
    let (mut payment, _) = prepare_payment(cost, pool, &mut supplies)?;
    let chosen = std::mem::take(&mut payment.source_choices);
    Some((payment, chosen))
}

/// Prepare a complete payment with minimum life loss, then minimum mana activations.
///
/// Each source offers one mana. Entries 0 through 5 are WUBRG and colorless.
/// `None` means that type is unavailable. Produced pool mana has zero source cost.
/// The prepared change spends only existing pool mana. Use the returned choices to
/// activate typed sources, then prepare the actual spell payment against that pool.
pub(in crate::deck::simulator) fn payment_plan_with_source_costs(
    cost: &Cost,
    pool: &ManaPool,
    classes: &[SpendRestriction],
    source_costs: &[[Option<u32>; 6]],
) -> Option<(ManaPayment, Vec<(usize, u8)>)> {
    payment_plan_with_cast_sources(
        cost,
        pool,
        classes,
        source_costs,
        &vec![None; source_costs.len()],
    )
}

/// Combine actual mana and convoke/improvise choices without reusing a physical source.
///
/// Each shared index is one permanent. `Some(mask)` offers a virtual generic payment
/// or one colored payment from bits 0 through 4; bit 5 is ignored. `None` offers no
/// virtual payment. Returned choices contain only mana activations, not virtual taps.
/// The prepared pool change does not tap virtual resources. After activating mana
/// modes, prepare the actual cast payment again against the resulting state.
pub(in crate::deck::simulator) fn payment_plan_with_cast_sources(
    cost: &Cost,
    pool: &ManaPool,
    classes: &[SpendRestriction],
    source_costs: &[[Option<u32>; 6]],
    virtual_taps: &[Option<u8>],
) -> Option<(ManaPayment, Vec<(usize, u8)>)> {
    assert_eq!(
        source_costs.len(),
        virtual_taps.len(),
        "each resource has one shared source index"
    );
    if cost.snow > 0 || cost.unsupported || cost.hybrid[0] > 0 {
        return None;
    }
    let mut supplies = supplies(pool, classes)?;
    supplies.extend(
        source_costs
            .iter()
            .enumerate()
            .filter_map(|(index, costs)| {
                let types = costs.iter().enumerate().fold(0, |types, (color, cost)| {
                    types | (u8::from(cost.is_some()) << color)
                }) | (virtual_taps[index].unwrap_or(0) & COLORED);
                (types != 0 || virtual_taps[index].is_some()).then_some(Supply {
                    amount: 1,
                    types,
                    generic: true,
                    origin: Origin::Virtual(index),
                    one_color: false,
                })
            }),
    );
    if !has_minimum_capacity(cost, &supplies) {
        return None;
    }
    let mut chosen = *cost;
    chosen.twobrid = [0; 5];
    let (mut payment, _) = choose_twobrid_with_source_costs(
        cost,
        &chosen,
        &mut supplies,
        pool,
        0,
        Some(SourcePrices {
            mana: source_costs,
            virtual_taps,
        }),
    )?;
    let selected = std::mem::take(&mut payment.source_choices);
    Some((payment, selected))
}

/// Reject unsupported symbols before choosing alternative symbol payments.
fn prepare_payment(
    cost: &Cost,
    pool: &ManaPool,
    supplies: &mut [Supply],
) -> Option<(ManaPayment, Vec<usize>)> {
    if cost.snow > 0 || cost.unsupported || cost.hybrid[0] > 0 {
        return None;
    }
    if *cost == Cost::default() {
        if supplies
            .iter()
            .any(|supply| supply.one_color && supply.types == 0)
        {
            return None;
        }
        let mut after = pool.clone();
        after.lots.retain(|lot| lot.amount > 0);
        return Some((
            ManaPayment {
                before: pool.clone(),
                after,
                source_choices: Vec::new(),
                source_cost: (0, 0),
            },
            Vec::new(),
        ));
    }
    if !has_minimum_capacity(cost, supplies) {
        return None;
    }
    let mut chosen = *cost;
    chosen.twobrid = [0; 5];
    choose_twobrid(cost, &chosen, supplies, pool, 0)
}

/// Reject shortages before exploring alternative symbols.
/// Twobrid needs at least one mana; callers pay Phyrexian life separately.
fn has_minimum_capacity(cost: &Cost, supplies: &[Supply]) -> bool {
    let required = u64::from(cost.generic)
        + u64::from(cost.colorless)
        + cost
            .pips
            .iter()
            .chain(&cost.hybrid)
            .chain(&cost.twobrid)
            .map(|count| u64::from(*count))
            .sum::<u64>();
    let available: u64 = supplies.iter().map(|supply| u64::from(supply.amount)).sum();
    available >= required
}

/// Enumerate only the five twobrid counts, not individual payment sequences.
fn choose_twobrid(
    cost: &Cost,
    chosen: &Cost,
    supplies: &mut [Supply],
    pool: &ManaPool,
    color: usize,
) -> Option<(ManaPayment, Vec<usize>)> {
    choose_twobrid_with_source_costs(cost, chosen, supplies, pool, color, None)
}

/// Compare alternative symbol payments when physical source types have different costs.
fn choose_twobrid_with_source_costs(
    cost: &Cost,
    chosen: &Cost,
    supplies: &mut [Supply],
    pool: &ManaPool,
    color: usize,
    source_costs: Option<SourcePrices<'_>>,
) -> Option<(ManaPayment, Vec<usize>)> {
    if color == 5 {
        return choose_single_color(chosen, supplies, pool, 0, source_costs);
    }
    let count = cost.twobrid[color];
    let mut best = None;
    for colored in (0..=count).rev() {
        let mut next = *chosen;
        let Some(pips) = next.pips[color].checked_add(colored) else {
            continue;
        };
        next.pips[color] = pips;
        let Some(generic) = next.generic.checked_add(2 * u32::from(count - colored)) else {
            continue;
        };
        next.generic = generic;
        let plan =
            choose_twobrid_with_source_costs(cost, &next, supplies, pool, color + 1, source_costs);
        if keep_cheaper_payment(&mut best, plan, source_costs.is_some()) {
            break;
        }
    }
    best
}

/// Preserve the common color of multi-mana, single-color source choices.
fn choose_single_color(
    cost: &Cost,
    supplies: &mut [Supply],
    pool: &ManaPool,
    start: usize,
    source_costs: Option<SourcePrices<'_>>,
) -> Option<(ManaPayment, Vec<usize>)> {
    let demands = demands(cost);
    let matcher = if let Some(costs) = source_costs {
        match_minimum_cost(supplies, &demands, costs)?
    } else {
        match_supplies(supplies, &demands)?
    };
    if matcher.bundles_consistent() {
        return Some(if let Some(costs) = source_costs {
            matcher.payment_with_source_costs(pool, costs)
        } else {
            matcher.payment(pool)
        });
    }
    let index = (start..supplies.len())
        .find(|i| supplies[*i].one_color && supplies[*i].types.count_ones() > 1)?;
    let types = supplies[index].types;
    let minimum = supplies[..index]
        .iter()
        .rev()
        .find(|earlier| same_bundle(earlier, &supplies[index], pool))
        .map_or(0, |earlier| earlier.types.trailing_zeros());
    let mut best = None;
    for bit in 0..6 {
        if bit < minimum || types & (1 << bit) == 0 {
            continue;
        }
        supplies[index].types = 1 << bit;
        let plan = choose_single_color(cost, supplies, pool, index + 1, source_costs);
        supplies[index].types = types;
        if keep_cheaper_payment(&mut best, plan, source_costs.is_some()) {
            break;
        }
    }
    best
}

/// Keep stable ties; a zero source cost cannot be improved by another assignment.
fn keep_cheaper_payment(
    best: &mut Option<(ManaPayment, Vec<usize>)>,
    candidate: Option<(ManaPayment, Vec<usize>)>,
    priced: bool,
) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    if best
        .as_ref()
        .is_none_or(|held| candidate.0.source_cost < held.0.source_cost)
    {
        *best = Some(candidate);
    }
    !priced
        || best
            .as_ref()
            .is_some_and(|held| held.0.source_cost == (0, 0))
}

/// Avoid permuting identical bundles between the same chosen colors.
fn same_bundle(first: &Supply, second: &Supply, pool: &ManaPool) -> bool {
    let (Origin::Lot(first), Origin::Lot(second)) = (first.origin, second.origin) else {
        return false;
    };
    pool.lots[first] == pool.lots[second] && pool.lots[first].one_color
}

/// Retain exact source locations while excluding ineligible cast classes.
fn supplies(pool: &ManaPool, classes: &[SpendRestriction]) -> Option<Vec<Supply>> {
    let mut supplies = Vec::with_capacity(pool.lots.len() + 14);
    for (index, lot) in pool.lots.iter().enumerate() {
        if lot
            .restriction
            .is_some_and(|class| !classes.contains(&class))
        {
            continue;
        }
        if lot.amount > 0 {
            supplies.push(Supply {
                amount: lot.amount,
                types: lot.types,
                generic: !lot.cannot_pay_generic,
                origin: Origin::Lot(index),
                one_color: lot.one_color,
            });
        }
    }
    for color in 0..5 {
        let no_generic = pool.fixed_no_generic[color];
        let unrestricted = pool.fixed[color].checked_sub(no_generic)?;
        for (amount, generic, origin) in [
            (no_generic, false, Origin::NoGeneric(color)),
            (unrestricted, true, Origin::Fixed(color)),
        ] {
            if amount > 0 {
                supplies.push(Supply {
                    amount,
                    types: 1 << color,
                    generic,
                    origin,
                    one_color: false,
                });
            }
        }
    }
    for (amount, types, origin) in [
        (pool.generic_credit, 0, Origin::Credit),
        (pool.colorless, COLORLESS, Origin::Colorless),
        (pool.flexible, COLORED, Origin::Flexible),
        (pool.treasures, COLORED, Origin::Treasure),
    ] {
        if amount > 0 {
            supplies.push(Supply {
                amount,
                types,
                generic: true,
                origin,
                one_color: false,
            });
        }
    }
    supplies.sort_by_key(|supply| match supply.types {
        0 => 0,
        COLORLESS => 1,
        _ => 2,
    });
    Some(supplies)
}

/// Group identical pip requirements; generic mana is matched last.
fn demands(cost: &Cost) -> Vec<Demand> {
    let mut demands = Vec::with_capacity(38);
    for color in 0..5 {
        if cost.pips[color] > 0 {
            demands.push(Demand {
                amount: u32::from(cost.pips[color]),
                types: 1 << color,
            });
        }
    }
    if cost.colorless > 0 {
        demands.push(Demand {
            amount: u32::from(cost.colorless),
            types: COLORLESS,
        });
    }
    for mask in 1..32 {
        if cost.hybrid[mask] > 0 {
            demands.push(Demand {
                amount: u32::from(cost.hybrid[mask]),
                types: mask as u8,
            });
        }
    }
    if cost.generic > 0 {
        demands.push(Demand {
            amount: cost.generic,
            types: 0,
        });
    }
    demands
}

/// Solve the capacitated matching, rerouting earlier assignments when needed.
fn match_supplies<'a>(supplies: &'a [Supply], demands: &'a [Demand]) -> Option<Matcher<'a>> {
    #[cfg(test)]
    MATCHER_CALLS.with(|calls| calls.set(calls.get() + 1));
    let mut matcher = Matcher {
        supplies,
        demands,
        remaining: supplies.iter().map(|supply| supply.amount).collect(),
        assigned: vec![vec![0; demands.len()]; supplies.len()],
    };
    if demands.is_empty() {
        return Some(matcher);
    }
    let mut visited = vec![false; supplies.len()];
    for (index, demand) in demands.iter().enumerate() {
        let mut needed = demand.amount;
        while needed > 0 {
            visited.fill(false);
            let paid = matcher.augment(index, needed, &mut visited);
            if paid == 0 {
                return None;
            }
            needed -= paid;
        }
    }
    Some(matcher)
}

/// Prefer virtual payment before an equal-life mana activation, then choose an actual type.
fn source_type_cost(
    supply: &Supply,
    allowed: u8,
    generic: bool,
    source_costs: SourcePrices<'_>,
) -> Option<SourceChoice> {
    let Origin::Virtual(source) = supply.origin else {
        return Some(SourceChoice {
            cost: (0, 0),
            mana_type: 0,
            activate: false,
        });
    };
    if let Some(types) = source_costs.virtual_taps[source] {
        let colored = allowed & types & COLORED;
        if generic || colored != 0 {
            return Some(SourceChoice {
                cost: (0, 0),
                mana_type: if generic {
                    0
                } else {
                    1 << colored.trailing_zeros()
                },
                activate: false,
            });
        }
    }
    source_costs.mana[source]
        .iter()
        .enumerate()
        .filter(|(color, _)| allowed & (1 << color) != 0)
        .filter_map(|(color, cost)| cost.map(|cost| (cost, color)))
        .min_by_key(|(cost, color)| (*cost, *color != 5, *color))
        .map(|(cost, color)| SourceChoice {
            cost: (i128::from(cost), 1),
            mana_type: 1 << color,
            activate: true,
        })
}

/// Find the minimum-cost capacitated assignment, including residual reroutes.
fn match_minimum_cost<'a>(
    supplies: &'a [Supply],
    demands: &'a [Demand],
    source_costs: SourcePrices<'_>,
) -> Option<Matcher<'a>> {
    let demand_start = supplies.len() + 1;
    let sink = demand_start + demands.len();
    let mut flow = CostFlow {
        edges: vec![Vec::new(); sink + 1],
    };
    let mut positions = vec![vec![None; demands.len()]; supplies.len()];
    for (source, supply) in supplies.iter().enumerate() {
        flow.add_edge(0, source + 1, supply.amount, (0, 0));
        for (index, demand) in demands.iter().enumerate() {
            let allowed = if demand.types == 0 {
                if !supply.generic {
                    continue;
                }
                supply.types
            } else {
                let allowed = supply.types & demand.types;
                if allowed == 0 {
                    continue;
                }
                allowed
            };
            let choice = source_type_cost(supply, allowed, demand.types == 0, source_costs)?;
            positions[source][index] = Some(flow.add_edge(
                source + 1,
                demand_start + index,
                supply.amount.min(demand.amount),
                choice.cost,
            ));
        }
    }
    for (index, demand) in demands.iter().enumerate() {
        flow.add_edge(demand_start + index, sink, demand.amount, (0, 0));
    }
    flow.fund(
        sink,
        demands.iter().map(|demand| u64::from(demand.amount)).sum(),
    )?;
    let mut matcher = Matcher {
        supplies,
        demands,
        remaining: supplies.iter().map(|supply| supply.amount).collect(),
        assigned: vec![vec![0; demands.len()]; supplies.len()],
    };
    for (source, edges) in positions.into_iter().enumerate() {
        for (demand, position) in edges.into_iter().enumerate() {
            if let Some(position) = position {
                let amount = supplies[source].amount.min(demands[demand].amount)
                    - flow.edges[source + 1][position].remaining;
                matcher.assigned[source][demand] = amount;
                matcher.remaining[source] -= amount;
            }
        }
    }
    Some(matcher)
}

/// One residual edge in the physical-source payment graph.
#[derive(Clone)]
struct CostEdge {
    to: usize,
    reverse: usize,
    remaining: u32,
    cost: SourceCost,
}

/// Capacitated payment graph with reverse edges for cheaper reassignment.
struct CostFlow {
    edges: Vec<Vec<CostEdge>>,
}

impl CostFlow {
    /// Attach an eligible mana-to-symbol edge and its residual reverse edge.
    fn add_edge(&mut self, from: usize, to: usize, amount: u32, cost: SourceCost) -> usize {
        let forward = self.edges[from].len();
        let reverse = self.edges[to].len();
        self.edges[from].push(CostEdge {
            to,
            reverse,
            remaining: amount,
            cost,
        });
        self.edges[to].push(CostEdge {
            to: from,
            reverse: forward,
            remaining: 0,
            cost: (-cost.0, -cost.1),
        });
        forward
    }

    /// Fund the entire demand without enumerating physical source combinations.
    fn fund(&mut self, sink: usize, mut needed: u64) -> Option<()> {
        if needed == 0 {
            return Some(());
        }
        let mut distance = vec![None; self.edges.len()];
        let mut previous = vec![None; self.edges.len()];
        while needed > 0 {
            self.shortest_paths(&mut distance, &mut previous);
            previous[sink]?;
            let mut amount = u32::try_from(needed.min(u64::from(u32::MAX)))
                .expect("bounded payment flow fits u32");
            let mut node = sink;
            while node != 0 {
                let (from, index) =
                    previous[node].expect("a reachable payment node has a predecessor");
                amount = amount.min(self.edges[from][index].remaining);
                node = from;
            }
            node = sink;
            while node != 0 {
                let (from, index) =
                    previous[node].expect("the prepared payment path remains reachable");
                let reverse = self.edges[from][index].reverse;
                self.edges[from][index].remaining -= amount;
                self.edges[node][reverse].remaining += amount;
                node = from;
            }
            needed -= u64::from(amount);
        }
        Some(())
    }

    /// Retain stable shortest paths while allowing negative-cost reverse edges.
    fn shortest_paths(
        &self,
        distance: &mut [Option<SourceCost>],
        previous: &mut [Option<(usize, usize)>],
    ) {
        distance.fill(None);
        previous.fill(None);
        distance[0] = Some((0i128, 0i128));
        for _ in 1..self.edges.len() {
            let mut changed = false;
            for (from, edges) in self.edges.iter().enumerate() {
                let Some(before) = distance[from] else {
                    continue;
                };
                for (index, edge) in edges.iter().enumerate() {
                    let next = (before.0 + edge.cost.0, before.1 + edge.cost.1);
                    if edge.remaining > 0 && distance[edge.to].is_none_or(|held| next < held) {
                        distance[edge.to] = Some(next);
                        previous[edge.to] = Some((from, index));
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }
}

/// Mutable residual assignments for an exact mana-to-symbol matching.
struct Matcher<'a> {
    supplies: &'a [Supply],
    demands: &'a [Demand],
    remaining: Vec<u32>,
    assigned: Vec<Vec<u32>>,
}

impl Matcher<'_> {
    /// Retain the minimum-cost actual type for every physical source used.
    fn payment_with_source_costs(
        self,
        pool: &ManaPool,
        source_costs: SourcePrices<'_>,
    ) -> (ManaPayment, Vec<usize>) {
        let mut choices = Vec::new();
        let mut total = (0, 0);
        for (index, supply) in self.supplies.iter().enumerate() {
            if let Origin::Virtual(source) = supply.origin
                && self.remaining[index] < supply.amount
            {
                let generic = self
                    .demands
                    .iter()
                    .enumerate()
                    .all(|(demand, need)| self.assigned[index][demand] == 0 || need.types == 0);
                let choice =
                    source_type_cost(supply, self.bundle_types(index), generic, source_costs)
                        .expect("each matched physical source has a priced actual type");
                total = (total.0 + choice.cost.0, total.1 + choice.cost.1);
                if choice.activate {
                    choices.push((source, choice.mana_type));
                }
            }
        }
        let (mut payment, used) = self.payment(pool);
        payment.source_choices = choices;
        payment.source_cost = total;
        (payment, used)
    }

    /// Find a common produced type for every symbol assigned to a bundle.
    fn bundle_types(&self, source: usize) -> u8 {
        self.demands.iter().enumerate().fold(
            self.supplies[source].types,
            |types, (demand, need)| {
                if self.assigned[source][demand] > 0 && need.types != 0 {
                    types & need.types
                } else {
                    types
                }
            },
        )
    }

    /// A relaxed matching is exact when no bundle needs two different types.
    fn bundles_consistent(&self) -> bool {
        self.supplies
            .iter()
            .enumerate()
            .all(|(source, supply)| !supply.one_color || self.bundle_types(source) != 0)
    }

    /// Build the atomic pool change from a matching with valid bundle choices.
    fn payment(self, pool: &ManaPool) -> (ManaPayment, Vec<usize>) {
        let mut after = pool.clone();
        let mut virtual_sources = Vec::new();
        let mut source_choices = Vec::new();
        for (index, supply) in self.supplies.iter().enumerate() {
            let spent = supply.amount - self.remaining[index];
            match supply.origin {
                Origin::Fixed(color) => after.fixed[color] -= spent,
                Origin::NoGeneric(color) => {
                    after.fixed[color] -= spent;
                    after.fixed_no_generic[color] -= spent;
                }
                Origin::Flexible => after.flexible -= spent,
                Origin::Colorless => after.colorless -= spent,
                Origin::Credit => after.generic_credit -= spent,
                Origin::Treasure => after.treasures -= spent,
                Origin::Virtual(source) => {
                    if spent > 0 {
                        let allowed = self.bundle_types(index);
                        let chosen = if allowed & COLORLESS != 0 {
                            COLORLESS
                        } else if allowed != 0 {
                            1 << allowed.trailing_zeros()
                        } else {
                            0
                        };
                        virtual_sources.push(source);
                        source_choices.push((source, chosen));
                    }
                }
                Origin::Lot(lot) => {
                    after.lots[lot].amount -= spent;
                    if spent > 0 && supply.one_color {
                        after.lots[lot].types = 1 << self.bundle_types(index).trailing_zeros();
                        after.lots[lot].one_color = false;
                    }
                }
            }
        }
        after.lots.retain(|lot| lot.amount > 0);
        (
            ManaPayment {
                before: pool.clone(),
                after,
                source_choices,
                source_cost: (0, 0),
            },
            virtual_sources,
        )
    }

    /// Find a residual path that funds more of one demand.
    fn augment(&mut self, demand: usize, needed: u32, visited: &mut [bool]) -> u32 {
        for source in 0..self.supplies.len() {
            let supply = &self.supplies[source];
            let types = self.demands[demand].types;
            let eligible = if types == 0 {
                supply.generic
            } else {
                supply.types & types != 0
            };
            if visited[source] || !eligible {
                continue;
            }
            visited[source] = true;
            let available = needed.min(self.remaining[source]);
            if available > 0 {
                self.remaining[source] -= available;
                self.assigned[source][demand] += available;
                return available;
            }
            for other in 0..self.demands.len() {
                let movable = needed.min(self.assigned[source][other]);
                if other == demand || movable == 0 {
                    continue;
                }
                let moved = self.augment(other, movable, visited);
                if moved > 0 {
                    self.assigned[source][other] -= moved;
                    self.assigned[source][demand] += moved;
                    return moved;
                }
            }
        }
        0
    }
}

#[cfg(test)]
#[path = "../tests/payment_tests.rs"]
mod payment_tests;

/// Independent payment checks for mixed quantities and alternative symbols.
#[cfg(test)]
#[path = "../tests/payment_reference_tests.rs"]
mod payment_reference_tests;

/// Banked Treasures remain distinct from produced mana during payment.
#[cfg(test)]
#[path = "../tests/payment_treasure_tests.rs"]
mod payment_treasure_tests;

/// Virtual cast resources share the exact matcher without entering the mana pool.
#[cfg(test)]
#[path = "tests/payment_virtual_tests.rs"]
mod payment_virtual_tests;

/// Empty-cost plans preserve the exact matcher result without building assignments.
#[cfg(test)]
#[path = "tests/payment_empty_tests.rs"]
mod payment_empty_tests;

/// Physical one-mana choices preserve colorless types and source identity.
#[cfg(test)]
#[path = "tests/payment_source_choice_tests.rs"]
mod payment_source_choice_tests;

/// Minimum-cost physical choices retain exact source identity and payment rules.
#[cfg(test)]
#[path = "tests/payment_source_cost_tests.rs"]
mod payment_source_cost_tests;

/// Cast resources and actual mana share one physical slot without changing virtual type rules.
#[cfg(test)]
#[path = "tests/payment_cast_source_tests.rs"]
mod payment_cast_source_tests;

/// Capacity pruning and scratch reuse preserve exact payments and source choices.
#[cfg(test)]
#[path = "tests/payment_work_tests.rs"]
mod payment_work_tests;
