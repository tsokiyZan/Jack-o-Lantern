use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use crate::entity::attributes::Modifier;
use crate::entity::attributes::ModifierOperation;
use pumpkin_data::{Block, BlockStateId, attributes::Attributes};

use crossbeam::atomic::AtomicCell;
use pumpkin_data::{
    damage::DamageType,
    data_component_impl::EquipmentSlot,
    entity::EntityStatus,
    entity::EntityType,
    sound::{Sound, SoundCategory},
    tag,
    tag::Taggable,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use pumpkin_util::random::RandomImpl;

use crate::entity::{
    as_entity_base, Entity, EntityBase,
    ai::{
        goal::{
            active_target::ActiveTargetGoal, chase_player::ChasePlayerGoal,
            look_around::RandomLookAroundGoal, look_at_entity::LookAtEntityGoal,
            melee_attack::MeleeAttackGoal, pick_up_block::PickUpBlockGoal,
            place_block::PlaceBlockGoal, reset_universal_anger::ResetUniversalAngerGoal,
            revenge::RevengeGoal, swim::SwimGoal,
            teleport_towards_player::TeleportTowardsPlayerGoal, wander_around::WanderAroundGoal,
        },
        pathfinder::node::PathType,
    },
    mob::{
        Mob, MobEntity,
        neutral::{NeutralData, NeutralMob},
    },
    player::Player,
};

const SPEED_BOOST: f64 = 0.15;
const ENDERMAN_SPEED_BOOST_ID: &str = "minecraft:attacking";

pub const ENDERMAN_EYE_HEIGHT: f64 = 2.55;
pub const ENDERMAN_BODY_Y_OFFSET: f64 = 1.45;

pub struct EndermanEntity {
    pub mob_entity: MobEntity,
    neutral_data: NeutralData,
    carried_block: AtomicCell<Option<BlockStateId>>,
    /// Synced CREEPY flag, the open-mouth pose. Anger lives in `NeutralMob`.
    creepy: AtomicBool,
    provoked: AtomicBool,
    speed_boosted: AtomicBool,
    /// Vanilla `targetChangeTime`: when the current target was set, 0 without one.
    target_change_time: AtomicI32,
}

impl EndermanEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let entity = Self {
            mob_entity,
            neutral_data: NeutralData::default(),
            carried_block: AtomicCell::new(None),
            creepy: AtomicBool::new(false),
            provoked: AtomicBool::new(false),
            speed_boosted: AtomicBool::new(false),
            target_change_time: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(entity);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        let mut navigator = mob_arc.mob_entity.own_navigation();
        navigator.set_mob_dimensions(0.6, 2.9);
        navigator.set_pathfinding_malus(PathType::Water, -1.0);
        drop(navigator);

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(0, SwimGoal::new(&mob_arc.mob_entity));
            goal_selector.add_goal(1, Box::new(ChasePlayerGoal::new(mob_arc.clone())));
            goal_selector.add_goal(2, Box::new(MeleeAttackGoal::new(1.0, false)));
            goal_selector.add_goal(
                7,
                Box::new(WanderAroundGoal::avoiding_water_with_probability(1.0, 0.0)),
            );
            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));
            goal_selector.add_goal(10, Box::new(PlaceBlockGoal::new(mob_arc.clone())));
            goal_selector.add_goal(11, Box::new(PickUpBlockGoal::new(mob_arc.clone())));

            target_selector.add_goal(1, Box::new(TeleportTowardsPlayerGoal::new(mob_arc.clone())));
            target_selector.add_goal(2, Box::new(RevengeGoal::new(true)));
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::ENDERMITE, true),
            );
            target_selector.add_goal(4, ResetUniversalAngerGoal::new(false));
        };

        mob_arc
    }

    /// Vanilla `Enderman.repeatedlyTryToTeleport`.
    fn repeatedly_try_to_teleport(&self) {
        if !self.mob_entity.living_entity.entity.is_passenger() {
            for _ in 0..64 {
                if self.teleport_randomly() {
                    return;
                }
            }
        }
    }

    pub fn teleport_randomly(&self) -> bool {
        let entity = &self.mob_entity.living_entity.entity;
        let pos = entity.pos.load();
        let (x, y, z) = {
            let mut rng = entity.random();
            (
                pos.x + (rng.next_f64() - 0.5) * 64.0,
                pos.y + f64::from(rng.next_bounded_i32(64) - 32),
                pos.z + (rng.next_f64() - 0.5) * 64.0,
            )
        };

        self.teleport_to(x, y, z)
    }

    pub fn teleport_towards(&self, target: &dyn EntityBase) -> bool {
        let entity = &self.mob_entity.living_entity.entity;
        let pos = entity.pos.load();
        let target_pos = target.get_entity().pos.load();

        // Vanilla `teleportTowards`: from the target's eyes to the enderman's middle.
        let dir = Vector3::new(
            pos.x - target_pos.x,
            (pos.y + ENDERMAN_BODY_Y_OFFSET) - target.get_entity().get_eye_y(),
            pos.z - target_pos.z,
        )
        .normalize();
        let (nx, ny, nz) = (dir.x, dir.y, dir.z);
        let (x, y, z) = {
            let mut rng = entity.random();
            (
                pos.x + (rng.next_f64() - 0.5) * 8.0 - nx * 16.0,
                pos.y + f64::from(rng.next_bounded_i32(16) - 8) - ny * 16.0,
                pos.z + (rng.next_f64() - 0.5) * 8.0 - nz * 16.0,
            )
        };

        self.teleport_to(x, y, z)
    }

    pub fn teleport_to(&self, x: f64, y: f64, z: f64) -> bool {
        let entity = &self.mob_entity.living_entity.entity;
        let origin = entity.pos.load();
        let world = entity.world.load();

        let min_y = f64::from(world.dimension.min_y);
        let max_y = f64::from(world.dimension.min_y + world.dimension.height - 1);
        let mut target_y = y.clamp(min_y, max_y);

        let block_x = x.floor() as i32;
        let mut block_y = target_y.floor() as i32;
        let block_z = z.floor() as i32;
        let ground_pos = loop {
            let below_pos = BlockPos::new(block_x, block_y - 1, block_z);
            let below_state = world.get_block_state(&below_pos);
            if below_state.is_solid() {
                break Some(below_pos);
            }
            if block_y <= world.dimension.min_y {
                break None;
            }
            block_y -= 1;
            target_y = block_y as f64;
        };
        let Some(ground_pos) = ground_pos else {
            return false;
        };

        if world
            .get_fluid(&ground_pos)
            .has_tag(&tag::Fluid::MINECRAFT_WATER)
        {
            return false;
        }

        let bb = BoundingBox::new_from_pos(x, target_y, z, &entity.entity_dimension.load());
        if !world.is_space_empty(bb) || world.contains_any_liquid(bb) {
            return false;
        }

        let new_pos = Vector3::new(x, target_y, z);

        for pos in &[origin, new_pos] {
            world.play_sound(Sound::EntityEndermanTeleport, SoundCategory::Hostile, pos);
        }

        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::entity::entity_teleport::EntityTeleportEvent::new(
                    entity.entity_id,
                    origin,
                    new_pos,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return false;
            }
        }

        entity.teleport(new_pos, None, None, &world);
        // `randomTeleport(.., showParticles = true)`: entity event 46, whose portal particles
        // every tracking client draws.
        world.send_entity_status(entity, EntityStatus::Teleport, None);

        self.mob_entity
            .with_navigation(|navigation, _| navigation.stop());

        true
    }

    pub fn set_target(&self, target: Option<Arc<dyn EntityBase>>) {
        let is_some = target.is_some();
        let mut mob_target = self
            .mob_entity
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *mob_target = target;
        drop(mob_target);

        let tick_count = self
            .mob_entity
            .living_entity
            .entity
            .tick_count
            .load(Ordering::Relaxed);
        self.target_change_time
            .store(if is_some { tick_count } else { 0 }, Ordering::Relaxed);
        if is_some {
            self.set_creepy(true);
            // Use attribute modifier instead of direct speed arithmetic
            if !self.speed_boosted.swap(true, Ordering::Relaxed) {
                let living = &self.mob_entity.living_entity;
                let modifier = Modifier {
                    id: ENDERMAN_SPEED_BOOST_ID.to_string(),
                    amount: SPEED_BOOST,
                    operation: ModifierOperation::Add,
                };

                living.update_attribute(&Attributes::MOVEMENT_SPEED, |inst| {
                    inst.add_or_replace_modifier(modifier);
                });

                crate::entity::attributes::send_attribute_updates_for_living(
                    living,
                    vec![Attributes::MOVEMENT_SPEED],
                );
            }
        } else {
            self.set_creepy(false);
            self.set_provoked(false);
            if self.speed_boosted.swap(false, Ordering::Relaxed) {
                let living = &self.mob_entity.living_entity;

                living.update_attribute(&Attributes::MOVEMENT_SPEED, |inst| {
                    inst.remove_modifier(ENDERMAN_SPEED_BOOST_ID);
                });

                crate::entity::attributes::send_attribute_updates_for_living(
                    living,
                    vec![Attributes::MOVEMENT_SPEED],
                );
            }
        }
    }

    pub fn set_creepy(&self, creepy: bool) {
        self.creepy.store(creepy, Ordering::Relaxed);
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(pumpkin_data::tracked_data::enderman::CREEPY, creepy);
    }

    pub fn is_creepy(&self) -> bool {
        self.creepy.load(Ordering::Relaxed)
    }

    pub fn set_provoked(&self, provoked: bool) {
        self.provoked.store(provoked, Ordering::Relaxed);
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(pumpkin_data::tracked_data::enderman::STARED_AT, provoked);
    }

    pub fn set_carried_block(&self, block_state: Option<BlockStateId>) {
        self.carried_block.store(block_state);
        let value = block_state.map_or(VarInt(0), |id| VarInt(id.as_u16() as i32));
        self.mob_entity
            .living_entity
            .entity
            .set_synced_data(pumpkin_data::tracked_data::enderman::CARRY_STATE, value);
    }

    pub fn get_carried_block(&self) -> Option<BlockStateId> {
        self.carried_block.load()
    }

    /// Vanilla `Enderman.isBeingStaredBy`: no gaze disguise on the head, and
    /// `isLookingAtMe(player, 0.025, true, false, getEyeY())`.
    pub fn is_player_staring(&self, player: &Player) -> bool {
        if let Ok(equipment) = player.living_entity.entity_equipment.try_lock()
            && let Some(head_stack) = equipment.equipment.get(&EquipmentSlot::HEAD)
            && head_stack
                .item
                .has_tag(&pumpkin_data::tag::Item::MINECRAFT_GAZE_DISGUISE_EQUIPMENT)
        {
            return false;
        }

        let entity = &self.mob_entity.living_entity.entity;
        let gaze_height = entity.get_eye_y();
        let player_entity = player.get_entity();

        // `getViewVector(1.0F).normalize()`: `calculateViewVector` with the `Mth` tables.
        let rx = player_entity.pitch.load() * (std::f32::consts::PI / 180.0);
        let ry = -player_entity.yaw.load() * (std::f32::consts::PI / 180.0);
        let (cos_y, sin_y) = (pumpkin_util::math::cos(ry), pumpkin_util::math::sin(ry));
        let (cos_x, sin_x) = (pumpkin_util::math::cos(rx), pumpkin_util::math::sin(rx));
        let look = Vector3::new(
            f64::from(sin_y * cos_x),
            f64::from(-sin_x),
            f64::from(cos_y * cos_x),
        )
        .normalize();

        let pos = entity.pos.load();
        let player_pos = player_entity.pos.load();
        let player_eye_y = player_entity.get_eye_y();
        let dir = Vector3::new(
            pos.x - player_pos.x,
            gaze_height - player_eye_y,
            pos.z - player_pos.z,
        );
        let dist = dir.length();
        let dir = dir.normalize();
        let dot = look.x * dir.x + look.y * dir.y + look.z * dir.z;
        if dot <= 1.0 - 0.025 / dist {
            return false;
        }

        // `player.hasLineOfSight(this, COLLIDER, NONE, gazeHeight)`: from the player's eyes.
        let from = Vector3::new(player_pos.x, player_eye_y, player_pos.z);
        let to = Vector3::new(pos.x, gaze_height, pos.z);
        if from.squared_distance_to_vec(&to).sqrt() > 128.0 {
            return false;
        }
        entity
            .world
            .load()
            .clip(&pumpkin_world::clip::ClipContext::new(
                from,
                to,
                pumpkin_world::clip::ClipBlock::Collider,
                pumpkin_world::clip::ClipFluid::None,
            ))
            .is_miss
    }
}

impl NeutralMob for EndermanEntity {
    fn get_neutral_data(&self) -> &NeutralData {
        &self.neutral_data
    }
}

impl Mob for EndermanEntity {
    /// Vanilla `EnderMan.getWalkTargetValue`.
    fn get_walk_target_value(&self, _pos: &pumpkin_util::math::position::BlockPos) -> f32 {
        0.0
    }

    /// Vanilla `Enderman.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        Some(if self.is_creepy() {
            pumpkin_data::sound::Sound::EntityEndermanScream
        } else {
            pumpkin_data::sound::Sound::EntityEndermanAmbient
        })
    }

    fn as_neutral(&self) -> Option<&dyn NeutralMob> {
        Some(self)
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        // Vanilla `BlockState.CODEC`.
        if let Some(block_state) = self.carried_block.load() {
            nbt.put("carriedBlockState", Block::state_to_nbt(block_state));
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        // Vanilla `BlockState.CODEC`, air read as nothing; older Pumpkin saves held a raw id.
        let carried = nbt
            .get("carriedBlockState")
            .and_then(|tag| match tag {
                NbtTag::Int(id) => BlockStateId::new(*id as u16),
                tag => Block::state_from_nbt(tag),
            })
            .filter(|state| !pumpkin_data::BlockState::from_id(*state).is_air());
        self.set_carried_block(carried);
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn set_mob_target(&self, target: Option<Arc<dyn EntityBase>>) {
        self.set_target(self.as_valid_target(target));
    }

    fn is_sensitive_to_water(&self) -> bool {
        true
    }

    /// Vanilla `Enderman.customServerAiStep`: in daylight, a while after picking a target, it
    /// may give up and teleport away.
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load();
        let tick_count = entity.tick_count.load(Ordering::Relaxed);
        if world.is_bright_outside()
            && tick_count >= self.target_change_time.load(Ordering::Relaxed) + 600
        {
            // `getLightLevelDependentMagicValue` at the eyes, `canSeeSky` at the feet.
            let pos = entity.pos.load();
            let eye =
                pumpkin_util::math::position::BlockPos::floored(pos.x, entity.get_eye_y(), pos.z);
            let br = world.get_light_level_dependent_magic_value(&eye);
            if br > 0.5
                && world.can_see_sky(&entity.block_pos.load())
                && entity.random().next_f32() * 30.0 < (br - 0.4) * 2.0
            {
                self.set_target(None);
                self.teleport_randomly();
            }
        }
    }

    /// Vanilla `Enderman.dropCustomDeathLoot`: the carried block's drops, as if mined with a
    /// diamond axe enchanted by the `enderman_loot_drop` provider.
    fn drop_custom_death_loot(&self) {
        let Some(state_id) = self.get_carried_block() else {
            return;
        };
        let entity = &self.mob_entity.living_entity.entity;
        let world = entity.world.load_full();
        let (block, state) = pumpkin_data::BlockState::from_id_with_block(state_id);
        let mut tool =
            pumpkin_data::item_stack::ItemStack::new(1, &pumpkin_data::item::Item::DIAMOND_AXE);
        crate::enchantment::EnchantmentHelper::enchant_item_from_provider(
            &mut tool,
            &pumpkin_data::enchantment_provider::EnchantmentProvider::ENDERMAN_LOOT_DROP,
            &crate::entity::mob::equipment::RegionalDifficulty::at(&world, entity.pos.load()),
            &mut *entity.random(),
        );
        let params = crate::world::loot::LootContextParameters {
            block_state: Some(state),
            position: Some(entity.pos.load()),
            tool: Some(tool),
            this_entity: Some(as_entity_base(&entity.entity_type)),
            ..Default::default()
        };
        let pos = entity.block_pos.load();
        for stack in crate::block::block_loot_stacks(&world, block, &pos, &params) {
            entity.spawn_at_location(stack);
        }
    }

    /// Vanilla `Enderman.hurtServer`.
    fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        let entity = &self.mob_entity.living_entity.entity;
        if entity.is_invulnerable_to(&damage_type, cause) {
            return false;
        }
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_PROJECTILE) && !entity.is_passenger()
        {
            self.repeatedly_try_to_teleport();
            return false;
        }
        if let Some(potion) =
            source.and_then(crate::entity::projectile::splash_potion::thrown_potion_item)
        {
            // `hurtWithCleanWater`.
            let hurt =
                crate::entity::projectile::splash_potion::hurts_water_sensitive_entities(&potion)
                    && crate::entity::mob::mob_hurt_server(
                        self,
                        caller,
                        amount,
                        damage_type,
                        position,
                        source,
                        cause,
                    );
            self.repeatedly_try_to_teleport();
            return hurt;
        }
        let result = crate::entity::mob::mob_hurt_server(
            self,
            caller,
            amount,
            damage_type,
            position,
            source,
            cause,
        );
        if cause.is_none_or(|cause| cause.get_living_entity().is_none())
            && self.get_entity().random().next_bounded_i32(10) != 0
        {
            self.teleport_randomly();
        }
        result
    }
}
