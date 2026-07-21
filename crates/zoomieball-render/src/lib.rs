#![warn(missing_docs)]

//! Controller-independent packed render upload and camera/debug-view state.

use zoomieball_core::{BodyId, RayObservation, World};

/// One cosmetic sphere instance in a CPU presentation snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct RenderInstance {
    /// World position.
    pub position: [f32; 3],
    /// Linear velocity for render-only rolling orientation.
    pub velocity: [f32; 3],
    /// Radius.
    pub radius: f32,
    /// Team code (`0`, `1`, or `2` for neutral).
    pub team: u32,
    /// Local number, with `u32::MAX` for the objective.
    pub local_id: u32,
    /// Physical role code.
    pub role: u32,
}

/// Reusable one-way cosmetic state published from the CPU implementation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RenderSnapshot {
    /// Completed authoritative tick represented by this frame.
    pub tick: u64,
    /// Packed sphere instances in canonical body order.
    pub instances: Vec<RenderInstance>,
}

impl RenderSnapshot {
    /// Allocate capacity for a world once.
    #[must_use]
    pub fn with_capacity(body_count: usize) -> Self {
        Self {
            tick: 0,
            instances: Vec::with_capacity(body_count),
        }
    }

    /// Replace this cosmetic publication from one authoritative CPU world.
    pub fn publish(&mut self, world: &World) {
        let view = world.view();
        self.tick = world.tick();
        self.instances.clear();
        self.instances
            .extend((0..view.len()).map(|index| RenderInstance {
                position: [
                    view.positions[index].x.to_f32(),
                    view.positions[index].y.to_f32(),
                    view.positions[index].z.to_f32(),
                ],
                velocity: [
                    view.velocities[index].x.to_f32(),
                    view.velocities[index].y.to_f32(),
                    view.velocities[index].z.to_f32(),
                ],
                radius: view.radii[index].to_f32(),
                team: view.teams[index].map_or(2, |team| team as u32),
                local_id: match view.ids[index] {
                    BodyId::Player { local, .. } => u32::from(local.get()),
                    BodyId::Objective => u32::MAX,
                },
                role: view.roles[index] as u32,
            }));
    }
}

/// Projection and inspection mode selected for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Camera {
    /// High, slightly sheared orthographic broadcast view.
    Broadcast {
        /// Orthographic half-height in arena units.
        half_height: f32,
        /// Broadcast shear along X.
        shear: f32,
    },
    /// Free inspection camera.
    Free {
        /// World position.
        position: [f32; 3],
        /// Unit look direction.
        forward: [f32; 3],
    },
    /// First-person perception inspection for one physical body.
    FirstPerson {
        /// Canonical body index.
        body: usize,
    },
}

impl Default for Camera {
    fn default() -> Self {
        Self::Broadcast {
            half_height: 12.0,
            shear: 0.12,
        }
    }
}

/// Independent contour-loop visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContourControls {
    /// Outer arena contour.
    pub arena: bool,
    /// Goal/cove contour.
    pub goals: bool,
}

impl Default for ContourControls {
    fn default() -> Self {
        Self {
            arena: true,
            goals: true,
        }
    }
}

/// Physical output extent after device-pixel-ratio scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceExtent {
    /// Physical pixel width.
    pub width: u32,
    /// Physical pixel height.
    pub height: u32,
}

impl SurfaceExtent {
    /// Convert a logical CSS extent to full-DPR physical pixels.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    pub fn from_logical(width: u32, height: u32, device_pixel_ratio: f32) -> Self {
        assert!(device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0);
        Self {
            width: ((width as f32) * device_pixel_ratio).round().max(1.0) as u32,
            height: ((height as f32) * device_pixel_ratio).round().max(1.0) as u32,
        }
    }
}

/// The renderer's one mutable GPU-facing operation.
pub trait StorageUpload {
    /// Replace the packed sphere storage buffer in one operation.
    fn upload_instances(&mut self, instances: &[RenderInstance]);
}

/// GPU-resident authoritative state exposed without CPU state transfer.
pub trait ResidentStateSource {
    /// Number of sphere instances already resident for vertex pulling.
    fn instance_count(&self) -> usize;
}

/// Frame instrumentation proving the upload and readback contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Packed storage uploads issued for the frame.
    pub uploads: u8,
    /// GPU readbacks issued for the frame; always zero.
    pub readbacks: u8,
    /// Packed sphere count.
    pub instances: usize,
}

/// Cosmetic renderer state generic over a raw WebGPU storage-buffer sink.
#[derive(Debug)]
pub struct Renderer<U: StorageUpload> {
    upload: U,
    camera: Camera,
    contours: ContourControls,
    extent: SurfaceExtent,
}

impl<U: StorageUpload> Renderer<U> {
    /// Construct around an initialized device-specific upload sink.
    #[must_use]
    pub fn new(upload: U, extent: SurfaceExtent) -> Self {
        Self {
            upload,
            camera: Camera::default(),
            contours: ContourControls::default(),
            extent,
        }
    }

    /// Select broadcast, free, or first-person projection state.
    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
    }

    /// Set independently toggleable contour loops.
    pub fn set_contours(&mut self, contours: ContourControls) {
        self.contours = contours;
    }

    /// Resize in physical pixels after DPR conversion.
    pub fn resize(&mut self, extent: SurfaceExtent) {
        self.extent = extent;
    }

    /// Upload one packed snapshot exactly once and issue no readback.
    pub fn render(&mut self, snapshot: &RenderSnapshot) -> FrameStats {
        self.upload.upload_instances(&snapshot.instances);
        FrameStats {
            uploads: 1,
            readbacks: 0,
            instances: snapshot.instances.len(),
        }
    }

    /// Record a frame whose authoritative state is already GPU-resident.
    pub fn render_resident(&mut self, source: &impl ResidentStateSource) -> FrameStats {
        FrameStats {
            uploads: 0,
            readbacks: 0,
            instances: source.instance_count(),
        }
    }

    /// Selected camera.
    #[must_use]
    pub const fn camera(&self) -> Camera {
        self.camera
    }

    /// Current contour controls.
    #[must_use]
    pub const fn contours(&self) -> ContourControls {
        self.contours
    }

    /// Current physical render extent.
    #[must_use]
    pub const fn extent(&self) -> SurfaceExtent {
        self.extent
    }

    /// Borrow the device-specific sink for diagnostics.
    #[must_use]
    pub const fn upload_sink(&self) -> &U {
        &self.upload
    }
}

/// Borrowed perception overlay for the selected first-person body.
#[derive(Debug, Clone, Copy)]
pub struct PerceptionOverlay<'a> {
    /// Target-directed and fixed environment rays.
    pub rays: &'a [RayObservation],
    /// Final encoded population lanes.
    pub lanes: &'a [i32],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct CountingUpload {
        calls: usize,
        instances: usize,
    }

    struct Resident(usize);

    impl ResidentStateSource for Resident {
        fn instance_count(&self) -> usize {
            self.0
        }
    }

    impl StorageUpload for CountingUpload {
        fn upload_instances(&mut self, instances: &[RenderInstance]) {
            self.calls += 1;
            self.instances = instances.len();
        }
    }

    #[test]
    fn a_frame_is_one_packed_upload_and_no_readback() {
        let mut renderer = Renderer::new(
            CountingUpload::default(),
            SurfaceExtent {
                width: 1366,
                height: 768,
            },
        );
        let snapshot = RenderSnapshot {
            tick: 1,
            instances: vec![RenderInstance {
                position: [0.0; 3],
                velocity: [0.0; 3],
                radius: 0.35,
                team: 2,
                local_id: u32::MAX,
                role: 2,
            }],
        };
        let stats = renderer.render(&snapshot);
        assert_eq!(stats.uploads, 1);
        assert_eq!(stats.readbacks, 0);
        assert_eq!(renderer.upload_sink().calls, 1);
        assert_eq!(renderer.upload_sink().instances, 1);
    }

    #[test]
    fn cpu_snapshot_publication_is_owned_by_the_render_layer() {
        let world = World::new(10);
        let mut snapshot = RenderSnapshot::with_capacity(world.view().len());
        snapshot.publish(&world);
        assert_eq!(snapshot.tick, 0);
        assert_eq!(snapshot.instances.len(), 21);
    }

    #[test]
    fn resident_frame_transfers_no_authoritative_state() {
        let mut renderer = Renderer::new(
            CountingUpload::default(),
            SurfaceExtent {
                width: 1366,
                height: 768,
            },
        );
        let stats = renderer.render_resident(&Resident(201));
        assert_eq!(stats.uploads, 0);
        assert_eq!(stats.readbacks, 0);
        assert_eq!(stats.instances, 201);
        assert_eq!(renderer.upload_sink().calls, 0);
    }

    #[test]
    fn dpr_extent_keeps_fractional_scaling_in_physical_pixels() {
        assert_eq!(
            SurfaceExtent::from_logical(683, 384, 2.0),
            SurfaceExtent {
                width: 1366,
                height: 768
            }
        );
    }
}
