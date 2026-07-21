//! Uncapped target-directed hemisphere observations and deterministic spatial indexing.

use crate::fixed::{Fx, Vec3Fx};
use crate::playbook::OracleIntentBatch;
use crate::world::{Role, Team, WorldView};

/// Semantic relation of an observation to its embodied observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Relation {
    /// Same team.
    Teammate = 0,
    /// Opposing team.
    Opponent = 1,
    /// Neutral objective.
    Neutral = 2,
    /// Arena surface.
    Arena = 3,
    /// Goal mouth.
    Goal = 4,
}

/// Versioned semantic, role, and squad tag carried by a ray hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SemanticTag {
    /// Observer-relative team relation.
    pub relation: Relation,
    /// Physical role where one exists.
    pub role: Role,
    /// Play-assigned squad for player hits.
    pub squad: u8,
}

impl SemanticTag {
    /// Arena surface tag.
    pub const ARENA: Self = Self {
        relation: Relation::Arena,
        role: Role::Objective,
        squad: 0,
    };
    /// Goal mouth tag.
    pub const GOAL: Self = Self {
        relation: Relation::Goal,
        role: Role::Objective,
        squad: 0,
    };
}

/// One target-directed or fixed environment observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RayObservation {
    /// Observer-local forward-hemisphere direction in world coordinates.
    pub direction: Vec3Fx,
    /// Surface depth along the ray.
    pub depth: Fx,
    /// Semantic feature tag.
    pub tag: SemanticTag,
    /// Canonical body index, absent for fixed arena rays.
    pub target: Option<usize>,
}

/// CSR observations: body `i` owns `rays[offsets[i]..offsets[i + 1]]`.
#[derive(Debug, Clone)]
pub struct ObservationBatch {
    /// CSR body offsets.
    pub offsets: Vec<usize>,
    /// Canonically emitted ray records.
    pub rays: Vec<RayObservation>,
    relative: Vec<Vec3Fx>,
    lengths: Vec<i128>,
    near_order: Vec<usize>,
}

impl PartialEq for ObservationBatch {
    fn eq(&self, other: &Self) -> bool {
        self.offsets == other.offsets && self.rays == other.rays
    }
}

impl Eq for ObservationBatch {}

impl ObservationBatch {
    /// Preallocate for every body seeing every other sphere plus eight environment rays.
    #[must_use]
    pub fn with_capacity(body_count: usize) -> Self {
        Self {
            offsets: Vec::with_capacity(body_count + 1),
            rays: Vec::with_capacity(body_count.saturating_mul(body_count.saturating_add(7))),
            relative: vec![Vec3Fx::ZERO; body_count],
            lengths: vec![0; body_count],
            near_order: Vec::with_capacity(body_count),
        }
    }

    /// Observations belonging to one body.
    #[must_use]
    pub fn for_body(&self, body: usize) -> &[RayObservation] {
        &self.rays[self.offsets[body]..self.offsets[body + 1]]
    }

    /// Build observations using count-sorted spatial-index traversal for occluders.
    pub fn build(
        &mut self,
        world: WorldView<'_>,
        intents: &OracleIntentBatch,
        index: &SpatialIndex,
    ) {
        self.build_internal(world, intents, Some(index));
    }

    /// Brute-force semantic oracle used by equivalence tests.
    pub fn build_brute_force(&mut self, world: WorldView<'_>, intents: &OracleIntentBatch) {
        self.build_internal(world, intents, None);
    }

    fn build_internal(
        &mut self,
        world: WorldView<'_>,
        intents: &OracleIntentBatch,
        index: Option<&SpatialIndex>,
    ) {
        assert_eq!(world.len(), intents.intents.len());
        self.offsets.clear();
        self.rays.clear();
        self.offsets.push(0);
        for observer in 0..world.len() {
            let Some(team) = world.teams[observer] else {
                self.offsets.push(self.rays.len());
                continue;
            };
            let forward = resolved_forward(world, intents, observer, team);
            for target in 0..world.len() {
                self.relative[target] = world.positions[target] - world.positions[observer];
                self.lengths[target] = raw_length_squared(self.relative[target]);
            }
            self.near_order.clear();
            self.near_order.extend(0..world.len());
            self.near_order.sort_unstable_by(|first, second| {
                self.lengths[*first]
                    .cmp(&self.lengths[*second])
                    .then_with(|| first.cmp(second))
            });
            for target in 0..world.len() {
                if target == observer {
                    continue;
                }
                let offset = self.relative[target];
                let occluded = index.map_or_else(
                    || {
                        occluded_brute_force(
                            world,
                            observer,
                            target,
                            &self.near_order,
                            &self.relative,
                            &self.lengths,
                        )
                    },
                    |index| index.occluded(world, observer, target, &self.relative, &self.lengths),
                );
                if offset.dot(forward).raw() < 0 || occluded {
                    continue;
                }
                let distance = offset.length();
                self.rays.push(RayObservation {
                    direction: offset.normalized(),
                    depth: (distance - world.radii[target]).clamp(Fx::ZERO, distance),
                    tag: tag_for(world, observer, target),
                    target: Some(target),
                });
            }
            append_environment_rays(&mut self.rays, forward, team);
            self.offsets.push(self.rays.len());
        }
    }
}

fn resolved_forward(
    world: WorldView<'_>,
    intents: &OracleIntentBatch,
    observer: usize,
    team: Team,
) -> Vec3Fx {
    let normal = world.contacts[observer].normal;
    let desired = intents.intents[observer].position - world.positions[observer];
    let tangent_desired = desired - normal * desired.dot(normal);
    if tangent_desired != Vec3Fx::ZERO {
        return tangent_desired.normalized();
    }
    let tangent_velocity =
        world.velocities[observer] - normal * world.velocities[observer].dot(normal);
    if tangent_velocity != Vec3Fx::ZERO {
        return tangent_velocity.normalized();
    }
    team.attack_axis()
}

fn occluded_brute_force(
    world: WorldView<'_>,
    observer: usize,
    target: usize,
    order: &[usize],
    relative: &[Vec3Fx],
    lengths: &[i128],
) -> bool {
    let ray_len_sq = lengths[target];
    order
        .iter()
        .copied()
        .take_while(|&candidate| lengths[candidate] < ray_len_sq)
        .any(|candidate| blocks_ray(world, observer, target, candidate, relative, lengths))
}

fn blocks_ray(
    world: WorldView<'_>,
    observer: usize,
    target: usize,
    candidate: usize,
    relative: &[Vec3Fx],
    lengths: &[i128],
) -> bool {
    if candidate == observer || candidate == target || lengths[candidate] >= lengths[target] {
        return false;
    }
    let projection = raw_dot(relative[candidate], relative[target]);
    if projection <= 0 {
        return false;
    }
    let perpendicular_scaled = lengths[candidate] * lengths[target] - projection * projection;
    let radius = i128::from(world.radii[candidate].raw());
    perpendicular_scaled <= radius * radius * lengths[target]
}

fn raw_dot(lhs: Vec3Fx, rhs: Vec3Fx) -> i128 {
    i128::from(lhs.x.raw()) * i128::from(rhs.x.raw())
        + i128::from(lhs.y.raw()) * i128::from(rhs.y.raw())
        + i128::from(lhs.z.raw()) * i128::from(rhs.z.raw())
}

fn raw_length_squared(vector: Vec3Fx) -> i128 {
    raw_dot(vector, vector)
}

fn tag_for(world: WorldView<'_>, observer: usize, target: usize) -> SemanticTag {
    let relation = match (world.teams[observer], world.teams[target]) {
        (_, None) => Relation::Neutral,
        (Some(observer), Some(target)) if observer == target => Relation::Teammate,
        (Some(_), Some(_)) => Relation::Opponent,
        (None, _) => unreachable!("only player bodies own observation ranges"),
    };
    SemanticTag {
        relation,
        role: world.roles[target],
        squad: world.squads[target],
    }
}

fn append_environment_rays(rays: &mut Vec<RayObservation>, forward: Vec3Fx, team: Team) {
    let environment = [
        (Vec3Fx::X, Fx::from_i32(16), SemanticTag::ARENA),
        (-Vec3Fx::X, Fx::from_i32(16), SemanticTag::ARENA),
        (Vec3Fx::Y, Fx::from_i32(9), SemanticTag::ARENA),
        (-Vec3Fx::Y, Fx::from_i32(9), SemanticTag::ARENA),
        (Vec3Fx::Z, Fx::from_i32(6), SemanticTag::ARENA),
        (-Vec3Fx::Z, Fx::from_i32(1), SemanticTag::ARENA),
        (team.attack_axis(), Fx::from_i32(16), SemanticTag::GOAL),
        (-team.attack_axis(), Fx::from_i32(16), SemanticTag::GOAL),
    ];
    rays.extend(
        environment
            .into_iter()
            .filter(|(direction, _, _)| direction.dot(forward).raw() >= 0)
            .map(|(direction, depth, tag)| RayObservation {
                direction,
                depth,
                tag,
                target: None,
            }),
    );
}

/// Deterministic counting-sorted uniform grid over the arena volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialIndex {
    counts: Vec<usize>,
    offsets: Vec<usize>,
    cursors: Vec<usize>,
    members: Vec<usize>,
    dimensions: [usize; 3],
    cell_size: Fx,
    origin: Vec3Fx,
}

impl SpatialIndex {
    /// Construct an initialized one-meter arena grid.
    #[must_use]
    pub fn new(body_capacity: usize) -> Self {
        let dimensions = [33, 19, 7];
        let cells = dimensions.into_iter().product();
        Self {
            counts: vec![0; cells],
            offsets: vec![0; cells + 1],
            cursors: vec![0; cells],
            members: Vec::with_capacity(body_capacity.saturating_mul(8)),
            dimensions,
            cell_size: Fx::ONE,
            origin: Vec3Fx::new(Fx::from_i32(-16), Fx::from_i32(-9), Fx::ZERO),
        }
    }

    /// Rebuild by counting then scattering bodies in canonical input order.
    pub fn rebuild(&mut self, world: WorldView<'_>) {
        self.counts.fill(0);
        for body in 0..world.len() {
            let (minimum, maximum) = self.covered_cells(world.positions[body], world.radii[body]);
            for z in minimum[2]..=maximum[2] {
                for y in minimum[1]..=maximum[1] {
                    for x in minimum[0]..=maximum[0] {
                        let cell = self
                            .cell_index([x, y, z])
                            .expect("covered cells are clamped into the grid");
                        self.counts[cell] += 1;
                    }
                }
            }
        }
        self.offsets[0] = 0;
        for cell in 0..self.counts.len() {
            self.offsets[cell + 1] = self.offsets[cell] + self.counts[cell];
        }
        self.cursors
            .copy_from_slice(&self.offsets[..self.counts.len()]);
        self.members.clear();
        self.members
            .resize(*self.offsets.last().expect("the grid has cells"), 0);
        for (body, &position) in world.positions.iter().enumerate() {
            let (minimum, maximum) = self.covered_cells(position, world.radii[body]);
            for z in minimum[2]..=maximum[2] {
                for y in minimum[1]..=maximum[1] {
                    for x in minimum[0]..=maximum[0] {
                        let cell = self
                            .cell_index([x, y, z])
                            .expect("covered cells are clamped into the grid");
                        self.members[self.cursors[cell]] = body;
                        self.cursors[cell] += 1;
                    }
                }
            }
        }
    }

    /// Counting-sorted body indices. Cell order is not an observation emission order.
    #[must_use]
    pub fn members(&self) -> &[usize] {
        &self.members
    }

    fn occluded(
        &self,
        world: WorldView<'_>,
        observer: usize,
        target: usize,
        relative: &[Vec3Fx],
        lengths: &[i128],
    ) -> bool {
        let mut cell = self.cell_coordinates(world.positions[observer]);
        let end = self.cell_coordinates(world.positions[target]);
        let direction = relative[target];
        let raw_direction = [direction.x.raw(), direction.y.raw(), direction.z.raw()];
        let step = raw_direction.map(i32::signum);
        let absolute = raw_direction.map(i32::saturating_abs);
        let position = world.positions[observer];
        let raw_position = [position.x.raw(), position.y.raw(), position.z.raw()];
        let raw_origin = [
            self.origin.x.raw(),
            self.origin.y.raw(),
            self.origin.z.raw(),
        ];
        let mut boundary_distance: [i32; 3] =
            std::array::from_fn(|axis| match step[axis].cmp(&0) {
                std::cmp::Ordering::Equal => i32::MAX,
                std::cmp::Ordering::Greater => {
                    raw_origin[axis] + (cell[axis] + 1) * self.cell_size.raw() - raw_position[axis]
                }
                std::cmp::Ordering::Less => {
                    raw_position[axis] - (raw_origin[axis] + cell[axis] * self.cell_size.raw())
                }
            });

        loop {
            if self
                .cell_members(cell)
                .any(|candidate| blocks_ray(world, observer, target, candidate, relative, lengths))
            {
                return true;
            }
            if cell == end {
                return false;
            }
            let next_axis = (0..3)
                .filter(|&axis| step[axis] != 0 && cell[axis] != end[axis])
                .min_by(|&left, &right| {
                    (i64::from(boundary_distance[left]) * i64::from(absolute[right]))
                        .cmp(&(i64::from(boundary_distance[right]) * i64::from(absolute[left])))
                        .then_with(|| left.cmp(&right))
                })
                .expect("a nonzero segment has at least one traversal axis");
            for axis in 0..3 {
                if step[axis] != 0
                    && cell[axis] != end[axis]
                    && i64::from(boundary_distance[axis]) * i64::from(absolute[next_axis])
                        == i64::from(boundary_distance[next_axis]) * i64::from(absolute[axis])
                {
                    cell[axis] += step[axis];
                    boundary_distance[axis] += self.cell_size.raw();
                }
            }
        }
    }

    fn cell_members(&self, cell: [i32; 3]) -> impl Iterator<Item = usize> + '_ {
        self.cell_index(cell).into_iter().flat_map(|index| {
            self.members[self.offsets[index]..self.offsets[index + 1]]
                .iter()
                .copied()
        })
    }

    fn covered_cells(&self, position: Vec3Fx, radius: Fx) -> ([i32; 3], [i32; 3]) {
        (
            self.cell_coordinates(position - Vec3Fx::splat(radius)),
            self.cell_coordinates(position + Vec3Fx::splat(radius)),
        )
    }

    fn cell_coordinates(&self, position: Vec3Fx) -> [i32; 3] {
        let relative = position - self.origin;
        let coordinate = |value: Fx, bound: usize| {
            let raw = value.raw().max(0) / self.cell_size.raw();
            raw.min(i32::try_from(bound - 1).expect("grid bound fits i32"))
        };
        let x = coordinate(relative.x, self.dimensions[0]);
        let y = coordinate(relative.y, self.dimensions[1]);
        let z = coordinate(relative.z, self.dimensions[2]);
        [x, y, z]
    }

    fn cell_index(&self, coordinates: [i32; 3]) -> Option<usize> {
        if coordinates
            .into_iter()
            .zip(self.dimensions)
            .any(|(coordinate, bound)| {
                coordinate < 0 || usize::try_from(coordinate).map_or(true, |value| value >= bound)
            })
        {
            return None;
        }
        let [x, y, z] = coordinates
            .map(|coordinate| usize::try_from(coordinate).expect("coordinates checked above"));
        Some((z * self.dimensions[1] + y) * self.dimensions[0] + x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playbook::OracleIntent;
    use crate::world::{LocalId, World};

    fn forward_intents(world: &World) -> OracleIntentBatch {
        OracleIntentBatch {
            intents: world
                .view()
                .positions
                .iter()
                .map(|&position| OracleIntent {
                    position: position + Vec3Fx::X,
                    spin: Vec3Fx::ZERO,
                })
                .collect(),
        }
    }

    #[test]
    fn every_unoccluded_candidate_including_a_distant_tiny_target_is_emitted() {
        let mut world = World::new(10);
        let observer = world.player_index(Team::Zero, LocalId::GOALIE).unwrap();
        for body in 0..world.view().len() {
            world.set_position(
                body,
                Vec3Fx::new(Fx::from_i32(-10), Fx::from_i32(8), Fx::ONE),
            );
        }
        world.set_position(observer, Vec3Fx::new(Fx::ZERO, Fx::ZERO, Fx::ONE));
        let target = world
            .player_index(Team::One, LocalId::new(9).unwrap())
            .unwrap();
        world.set_position(
            target,
            Vec3Fx::new(Fx::from_i32(15), Fx::from_i32(7), Fx::ONE),
        );
        let intents = forward_intents(&world);
        let mut index = SpatialIndex::new(world.view().len());
        index.rebuild(world.view());
        let mut observations = ObservationBatch::with_capacity(world.view().len());
        observations.build(world.view(), &intents, &index);
        assert!(
            observations
                .for_body(observer)
                .iter()
                .any(|ray| ray.target == Some(target))
        );
    }

    #[test]
    fn nearer_collinear_sphere_occludes_and_hemisphere_boundary_is_inclusive() {
        let mut world = World::new(10);
        let observer = 0;
        let blocker = 1;
        let hidden = 2;
        let boundary = 3;
        world.set_position(observer, Vec3Fx::new(Fx::ZERO, Fx::ZERO, Fx::ONE));
        world.set_position(blocker, Vec3Fx::new(Fx::from_i32(2), Fx::ZERO, Fx::ONE));
        world.set_position(hidden, Vec3Fx::new(Fx::from_i32(4), Fx::ZERO, Fx::ONE));
        world.set_position(boundary, Vec3Fx::new(Fx::ZERO, Fx::from_i32(4), Fx::ONE));
        let intents = forward_intents(&world);
        let mut index = SpatialIndex::new(world.view().len());
        index.rebuild(world.view());
        let mut observations = ObservationBatch::with_capacity(world.view().len());
        observations.build(world.view(), &intents, &index);
        let rays = observations.for_body(observer);
        assert!(rays.iter().any(|ray| ray.target == Some(blocker)));
        assert!(!rays.iter().any(|ray| ray.target == Some(hidden)));
        assert!(rays.iter().any(|ray| ray.target == Some(boundary)));
    }

    #[test]
    fn counting_sorted_grid_matches_the_brute_force_oracle() {
        let world = World::new(10);
        let intents = forward_intents(&world);
        let mut index = SpatialIndex::new(world.view().len());
        index.rebuild(world.view());
        let mut grid = ObservationBatch::with_capacity(world.view().len());
        let mut brute = ObservationBatch::with_capacity(world.view().len());
        grid.build(world.view(), &intents, &index);
        brute.build_brute_force(world.view(), &intents);
        assert_eq!(grid, brute);
    }

    #[test]
    fn radius_expanded_grid_matches_brute_force_across_diagonal_fixtures() {
        let mut world = World::new(10);
        let mut index = SpatialIndex::new(world.view().len());
        let mut grid = ObservationBatch::with_capacity(world.view().len());
        let mut brute = ObservationBatch::with_capacity(world.view().len());
        for seed in 0..16usize {
            for body in 0..world.view().len() {
                let x = -15 * Fx::ONE_RAW
                    + i32::try_from((body * 977 + seed * 131) % 3_000).unwrap() * Fx::ONE_RAW / 100;
                let y = -8 * Fx::ONE_RAW
                    + i32::try_from((body * 577 + seed * 263) % 1_600).unwrap() * Fx::ONE_RAW / 100;
                let z = Fx::ONE_RAW / 2
                    + i32::try_from((body * 271 + seed * 89) % 500).unwrap() * Fx::ONE_RAW / 100;
                world.set_position(
                    body,
                    Vec3Fx::new(Fx::from_raw(x), Fx::from_raw(y), Fx::from_raw(z)),
                );
            }
            let intents = forward_intents(&world);
            index.rebuild(world.view());
            grid.build(world.view(), &intents, &index);
            brute.build_brute_force(world.view(), &intents);
            assert_eq!(grid, brute, "fixture {seed}");
        }
    }
}
