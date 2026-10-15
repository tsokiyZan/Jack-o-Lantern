//! Vanilla `Slime`'s spawn rules; the mob itself is `cube_mob`.

use pumpkin_util::Difficulty;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::random::RandomImpl;
use rand::RngExt;

use crate::world::World;

pub fn check_slime_spawn_rules(world: &World, pos: &BlockPos) -> bool {
    if world.level_info.load().difficulty == Difficulty::Peaceful {
        return false;
    }

    // TODO: check spawn reason. if it's spawner, we should return true if block below is valid
    // For now, we assume natural spawning as that's what we are implementing.

    // Swamp/Surface Spawning
    // TODO: fix
    // let biome = world.get_biome(pos);
    // if biome.has_tag(&pumpkin_data::tag::WorldgenBiome::MINECRAFT_ALLOWS_SURFACE_SLIME_SPAWNS)
    //     && pos.0.y > 50
    //     && pos.0.y < 70
    // {
    //     let time = world.level_time.lock().await.time_of_day;
    //     let moon_phase = (time / 24000) % 8;
    //     let surface_slime_spawn_chance = Self::get_spawn_chance(moon_phase);
    //     let mut rng = rand::rng();
    //     if rng.random::<f32>() < surface_slime_spawn_chance
    //         && world.get_max_local_raw_brightness(pos) <= rng.random_range(0..8)
    //     {
    //         return true;
    //     }
    // }

    // Slime Chunk Spawning
    let chunk_pos = pos.chunk_position();
    let world_seed = world.level.seed.0;
    let slime_seed =
        pumpkin_util::random::seed_slime_chunk(chunk_pos.x, chunk_pos.y, world_seed, 987_234_911);
    let mut slime_rand = pumpkin_util::random::legacy_rand::LegacyRand::from_seed(slime_seed);

    let mut rng = rand::rng();
    if rng.random_range(0..10) == 0 && slime_rand.next_bounded_i32(10) == 0 && pos.0.y < 40 {
        return true;
    }

    false
}

// const fn get_spawn_chance(moon_phase: i64) -> f32 {
//     match moon_phase {
//         0 => 1.0,
//         1 | 7 => 0.75,
//         2 | 6 => 0.5,
//         3 | 5 => 0.25,
//         _ => 0.0,
//     }
// }
