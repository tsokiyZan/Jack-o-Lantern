use std::sync::atomic::Ordering;

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector3::Vector3;

use crate::enchantment::EnchantmentHelper;
use crate::entity::damage_source::DamageSource;
use crate::{
    block::blocks::fire::FireBlockBase,
    entity::{
        Entity, EntityBase,
        projectile::{ProjectileHit, fireball::Fireball},
    },
    server::Server,
};

/// Vanilla `SmallFireball`: the blaze's and the dispenser's fireball, which sets fire instead
/// of exploding.
pub struct SmallFireballEntity {
    pub fireball: Fireball,
}

impl SmallFireballEntity {
    #[must_use]
    pub fn new(entity: Entity) -> Self {
        Self {
            fireball: Fireball::new(entity),
        }
    }

    /// Vanilla `SmallFireball(level, x, y, z, direction)`, at the entity's position.
    #[must_use]
    pub fn new_directional(entity: Entity, direction: Vector3<f64>) -> Self {
        Self {
            fireball: Fireball::new_directional(entity, direction),
        }
    }

    /// Vanilla `SmallFireball(level, mob, direction)`.
    #[must_use]
    pub fn new_from_mob(entity: Entity, mob: &Entity, direction: Vector3<f64>) -> Self {
        Self {
            fireball: Fireball::new_from_mob(entity, mob, direction),
        }
    }
}

impl EntityBase for SmallFireballEntity {
    fn get_owner_id(&self) -> Option<i32> {
        self.fireball.thrown.owner_id.load()
    }

    fn set_owner_id(&self, owner: Option<i32>) {
        self.fireball.thrown.owner_id.store(owner);
    }

    fn write_custom_nbt(&self, nbt: &mut NbtCompound) {
        self.fireball.write_nbt(nbt);
    }

    fn read_custom_nbt(&self, nbt: &NbtCompound) {
        self.fireball.read_nbt(nbt);
    }

    fn init_data_tracker(&self) {
        self.fireball.init_data_tracker();
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.fireball.tick(caller, server);
    }

    fn get_entity(&self) -> &Entity {
        self.fireball.get_entity()
    }

    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        let world = self.get_entity().world.load();
        let owner = self
            .get_owner_id()
            .and_then(|id| world.get_entity_by_id(id));
        match hit {
            // Vanilla `SmallFireball.onHitEntity`: the fire stays only if the hit lands.
            ProjectileHit::Entity { ref entity, .. } => {
                let target = entity.get_entity();
                let remaining_fire_ticks = target.fire_ticks.load(Ordering::Relaxed);
                entity.set_on_fire_for(5.0);
                let this = world.get_entity_by_id(self.get_entity().entity_id);
                let damage_source = DamageSource {
                    damage_type: Fireball::damage_type(owner.as_deref()),
                    direct: this.as_deref(),
                    // `fireball`: an unowned fireball is its own cause.
                    causing: owner.as_deref().or(this.as_deref()),
                };
                if entity.damage_with_context(
                    entity.as_ref(),
                    5.0,
                    damage_source.damage_type,
                    None,
                    damage_source.direct,
                    damage_source.causing,
                ) {
                    EnchantmentHelper::do_post_attack_effects(
                        &world,
                        entity.as_ref(),
                        &damage_source,
                    );
                } else {
                    target
                        .fire_ticks
                        .store(remaining_fire_ticks, Ordering::Relaxed);
                }
            }
            // Vanilla `SmallFireball.onHitBlock`: a mob's fireball only lights with mob griefing.
            ProjectileHit::Block { pos, face, .. } => {
                let mob_owned = owner
                    .as_ref()
                    .is_some_and(|owner| owner.get_mob().is_some());
                if !mob_owned || world.level_info.load().game_rules.mob_griefing {
                    let fire_pos = pos.offset(face.to_offset());
                    if world.get_block_state(&fire_pos).is_air() {
                        world.set_block_state(
                            &fire_pos,
                            FireBlockBase::get_state(&world, &fire_pos),
                            pumpkin_world::world::BlockFlags::NOTIFY_ALL,
                        );
                    }
                }
            }
        }
    }
}
