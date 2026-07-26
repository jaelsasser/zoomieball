//! Test-only fixtures the witness and checkpoint suites share: the shipped playbook, the
//! single-word member edit both mutation suites drive, the rule-dial swap that separates a pool's
//! learning dials from its stored words, and the counting global allocator the hot-path allocation
//! pin measures against.
//!
//! The allocator is registered for the whole test binary because a `GlobalAlloc` impl is the only
//! place an allocation can be observed at all, and its counter is thread-local so that the harness
//! running tests in parallel never charges one test's allocations to another's window.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use zoomie_core::NetId;
use zoomie_pop::{
    ExploratoryHebbParams, ExploratoryHebbRule, Population, SparseCtrnn, SparseCtrnnMember,
};
use zoomieball_core::Playbook;

/// The shipped default playbook every match fixture compiles.
pub(crate) fn playbook() -> Playbook {
    Playbook::compile_ron(include_str!("../../../assets/default-playbook.ron"))
        .expect("the shipped playbook compiles")
}

/// Rewrite one resident member through the pool's own extract/insert boundary, so a single-word
/// edit lands exactly as a live write would — family validation included, no upstream API widened.
pub(crate) fn mutate_member(
    pool: &mut Population<SparseCtrnn>,
    id: NetId,
    edit: impl FnOnce(&mut SparseCtrnnMember),
) {
    let mut member = pool.extract(id).expect("the edited identity is resident");
    edit(&mut member);
    pool.remove_batch(&[id]);
    pool.insert_batch(vec![(id, member)])
        .expect("a single-word edit leaves the member valid");
}

/// Rearm `pool` with a different exploration seed, carrying every resident member across word for
/// word.
///
/// The dials are pool-level, not member words, so this is the only way to hold two backends'
/// stored state bit-identical while their trajectories differ — which is exactly the state the
/// learning witness has to separate, and exactly what a restore has to adopt from a checkpoint.
pub(crate) fn rearm(pool: &mut Population<SparseCtrnn>, exploration_seed: u64) {
    let config = *pool.config();
    let members: Vec<_> = pool
        .ids()
        .iter()
        .map(|&id| (id, pool.extract(id).expect("a resident identity extracts")))
        .collect();
    let rule = ExploratoryHebbRule::new(
        ExploratoryHebbParams {
            exploration_seed,
            ..ExploratoryHebbParams::default()
        },
        &config,
    )
    .expect("the rearmed dials are valid");
    let mut rearmed = Population::new(pool.spec().clone(), config, Some(rule));
    rearmed
        .insert_batch(members)
        .expect("members valid under their own spec stay valid under new dials");
    *pool = rearmed;
}

/// Heap allocations this thread charged while `body` ran.
pub(crate) fn allocations(body: impl FnOnce()) -> u64 {
    let before = ALLOCATIONS.get();
    body();
    ALLOCATIONS.get() - before
}

thread_local! {
    /// `const`-initialized and drop-free, so touching it from inside the allocator neither
    /// allocates on first use nor re-enters during thread teardown.
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

/// Counts every heap request this thread makes, then defers to the system allocator.
struct CountingAllocator;

// The only way to observe allocation is to be the allocator, and that is an `unsafe impl` by
// definition; every method here is a pass-through to `System` with one counter bump ahead of it.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.set(ALLOCATIONS.get() + 1);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.set(ALLOCATIONS.get() + 1);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.set(ALLOCATIONS.get() + 1);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
