//! Public CPU tracer through controller learning and render publication.

use zoomieball_controller::ZoomieBackend;
use zoomieball_core::{ControllerBackend, Match, MatchConfig, Playbook};
use zoomieball_render::{RenderInstance, RenderSnapshot, Renderer, StorageUpload, SurfaceExtent};

#[derive(Debug, Default)]
struct CountingUpload {
    calls: usize,
}

impl StorageUpload for CountingUpload {
    fn upload_instances(&mut self, _instances: &[RenderInstance]) {
        self.calls += 1;
    }
}

#[test]
fn public_cpu_tracer_reaches_learning_witnesses_and_presentation() {
    let playbook =
        Playbook::compile_ron(include_str!("../../../assets/default-playbook.ron")).unwrap();
    let controller = ZoomieBackend::new(10, 0x005a_001e_ba11);
    let mut game = Match::new(MatchConfig::default(), playbook, controller);
    assert!(game.traverse_play(0));

    let mut snapshot = RenderSnapshot::with_capacity(game.world().view().len());
    let mut renderer = Renderer::new(
        CountingUpload::default(),
        SurfaceExtent {
            width: 1366,
            height: 768,
        },
    );
    for _ in 0..4 {
        game.tick();
        snapshot.publish(game.world());
        let frame = renderer.render(&snapshot);
        assert_eq!(frame.uploads, 1);
        assert_eq!(frame.readbacks, 0);
    }

    let witnesses = game.last_hash();
    assert_eq!(game.play_node(), 1);
    assert_eq!(game.world().tick(), 4);
    assert!(!game.observations().for_body(0).is_empty());
    assert_ne!(witnesses.physics, 0);
    assert_ne!(witnesses.controller, 0);
    assert_ne!(witnesses.learning, 0);
    assert_ne!(witnesses.pipeline, 0);
    assert_eq!(snapshot.instances.len(), 21);
    assert_eq!(renderer.upload_sink().calls, 4);

    let mut checkpoint = Vec::new();
    game.controller().checkpoint(&mut checkpoint);
    assert!(!checkpoint.is_empty());
}
