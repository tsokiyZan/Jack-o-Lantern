use std::sync::Arc;

use super::World;

/// An overworld in a temporary folder, removed when the `TempDir` drops.
pub fn test_world() -> (tempfile::TempDir, Arc<World>) {
    let folder = tempfile::tempdir().unwrap();
    let level = pumpkin_world::dimension::into_level(
        pumpkin_data::dimension::Dimension::OVERWORLD,
        &pumpkin_config::world::LevelConfig::default(),
        folder.path().to_path_buf(),
        0,
    );
    let level_info = Arc::new(arc_swap::ArcSwap::from_pointee(
        pumpkin_world::world_info::LevelData::default(pumpkin_util::world_seed::Seed(0)),
    ));
    let world = Arc::new(World::load(
        level,
        level_info,
        pumpkin_data::dimension::Dimension::OVERWORLD,
        crate::block::registry::default_registry(),
        std::sync::Weak::new(),
    ));
    (folder, world)
}
