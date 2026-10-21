use std::sync::{
    Arc, Weak,
    atomic::{AtomicU8, Ordering},
};

use pumpkin_data::entity::EntityPose;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::wolf_sound_variant::WolfSoundVariant;
use pumpkin_data::wolf_variant::WolfVariant;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::math::boundingbox::EntityDimensions;
use pumpkin_util::random::RandomImpl;

use crate::entity::ai::pathfinder::node::PathType;
use crate::entity::custom_sound::CustomSound;
use crate::entity::{
    Entity, EntityBase,
    ageable::AgeableMob,
    ai::behavior::neutral::apply_targets,
    ai::goal::{
        active_target::{ActiveTargetGoal, NonTameRandomTargetGoal},
        avoid_entity::AvoidEntityGoal,
        beg::BegGoal,
        breed::BreedGoal,
        escape_danger::EscapeDangerGoal,
        follow_owner::FollowOwnerGoal,
        leap_at_target::LeapAtTargetGoal,
        look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal,
        melee_attack::MeleeAttackGoal,
        owner_hurt_by_target::OwnerHurtByTargetGoal,
        owner_hurt_target::OwnerHurtTargetGoal,
        revenge::RevengeGoal,
        sit_when_ordered_to::SitWhenOrderedToGoal,
        swim::SwimGoal,
        wander_around::WanderAroundGoal,
    },
    mob::{
        Mob, MobEntity,
        neutral::{NeutralData, NeutralMob},
    },
    passive::{
        animal::{Animal, set_animal_pathfinding_malus},
        tamable::{TamableAnimal, TamableData},
    },
    player::Player,
};

pub struct WolfEntity {
    pub mob_entity: MobEntity,
    pub variant: AtomicU8,
    pub sound_variant: AtomicU8,
    pub collar_color: AtomicU8,
    pub tamable_data: TamableData,
    pub ageable_data: crate::entity::ageable::AgeableData,
    pub neutral_data: NeutralData,
}

impl WolfEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        set_animal_pathfinding_malus(&mob_entity);
        // Vanilla `Wolf`'s constructor.
        mob_entity.set_pathfinding_malus(PathType::PowderSnow, -1.0);
        mob_entity.set_pathfinding_malus(PathType::DangerPowderSnow, -1.0);
        let wolf = Self {
            mob_entity,
            variant: AtomicU8::new(WolfVariant::Pale.id()),
            sound_variant: AtomicU8::new(WolfSoundVariant::Classic as u8),
            collar_color: AtomicU8::new(14), // Default to red
            tamable_data: TamableData::default(),
            ageable_data: crate::entity::ageable::AgeableData::default(),
            neutral_data: NeutralData::default(),
        };
        let mob_arc = Arc::new(wolf);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            // Goal selector (matching Vanilla registerGoals):
            // 1: SwimGoal (FloatGoal)
            goal_selector.add_goal(1, SwimGoal::new(&mob_arc.mob_entity));
            // 1: EscapeDangerGoal (TamableAnimalPanicGoal)
            goal_selector.add_goal(1, EscapeDangerGoal::new(1.5));
            goal_selector.add_goal(2, Box::new(SitWhenOrderedToGoal::new()));
            // 3: Avoid Llama
            goal_selector.add_goal(
                3,
                Box::new(AvoidEntityGoal::new(&EntityType::LLAMA, 24.0, 1.5, 1.5)),
            );
            goal_selector.add_goal(4, Box::new(LeapAtTargetGoal::new(0.4)));
            // 5: MeleeAttackGoal
            goal_selector.add_goal(5, Box::new(MeleeAttackGoal::new(1.0, true)));
            // 6: FollowOwnerGoal
            goal_selector.add_goal(6, FollowOwnerGoal::new(1.0, 10.0, 2.0));
            // 7: BreedGoal
            goal_selector.add_goal(7, BreedGoal::new(1.0));
            // 8: WaterAvoidingRandomStrollGoal
            goal_selector.add_goal(8, Box::new(WanderAroundGoal::avoiding_water(1.0)));
            // 9: BegGoal
            goal_selector.add_goal(9, BegGoal::new(8.0));
            // 10: LookAtPlayer & RandomLookAround
            goal_selector.add_goal(
                10,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(10, Box::new(RandomLookAroundGoal::default()));
        };

        {
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            // Target selector (matching Vanilla registerGoals):
            // 1: OwnerHurtByTargetGoal
            target_selector.add_goal(1, OwnerHurtByTargetGoal::new());
            // 2: OwnerHurtTargetGoal
            target_selector.add_goal(2, OwnerHurtTargetGoal::new());
            // 3: HurtByTargetGoal (RevengeGoal)
            target_selector.add_goal(3, Box::new(RevengeGoal::new(true).alerting_others()));
            // 4: NearestAttackableTarget (Player, angry only), 8: ResetUniversalAnger
            apply_targets(&mut target_selector, &mob_arc.mob_entity, 4, 8, true);
            // 5: NonTameRandomTarget (Animal, `PREY_SELECTOR`); 6: baby turtles on land.
            target_selector.add_goal(
                5,
                NonTameRandomTargetGoal::new(ActiveTargetGoal::predicated(
                    &mob_arc.mob_entity,
                    10,
                    false,
                    |target, _| {
                        let entity_type = target.entity.entity_type;
                        entity_type == &EntityType::SHEEP
                            || entity_type == &EntityType::RABBIT
                            || entity_type == &EntityType::FOX
                    },
                )),
            );
            target_selector.add_goal(
                6,
                NonTameRandomTargetGoal::new(Box::new(ActiveTargetGoal::new(
                    &mob_arc.mob_entity,
                    &EntityType::TURTLE,
                    10,
                    false,
                    false,
                    Some(crate::entity::passive::cat::baby_on_land),
                ))),
            );
            // 7: NearestAttackableTarget (AbstractSkeleton)
            target_selector.add_goal(
                7,
                ActiveTargetGoal::predicated(&mob_arc.mob_entity, 10, false, |target, _| {
                    is_abstract_skeleton(target.entity.entity_type)
                }),
            );
        };

        mob_arc
    }

    pub fn get_tame_flags(&self) -> u8 {
        let mut flags = 0u8;
        if self.is_in_sitting_pose() {
            flags |= 0x01;
        }
        if self.is_tame() {
            flags |= 0x04;
        }
        flags
    }
}

impl AgeableMob for WolfEntity {
    fn get_ageable_data(&self) -> &crate::entity::ageable::AgeableData {
        &self.ageable_data
    }
}

impl Animal for WolfEntity {
    fn is_food(&self, item_stack: &ItemStack) -> bool {
        let item = item_stack.get_item();
        item.has_tag(&tag::Item::MINECRAFT_WOLF_FOOD) || item == &Item::BONE
    }

    /// Vanilla `Wolf.canMate`.
    fn can_mate(&self, partner: &dyn Mob) -> bool {
        if partner.get_entity().entity_id == self.get_entity().entity_id || !self.is_tame() {
            return false;
        }
        let Some(wolf) = partner
            .as_tamable()
            .filter(|_| partner.get_entity().entity_type == &EntityType::WOLF)
        else {
            return false;
        };
        wolf.is_tame()
            && !wolf.is_in_sitting_pose()
            && self.mob_entity.is_in_love()
            && partner.get_mob_entity().is_in_love()
    }
}

impl TamableAnimal for WolfEntity {
    fn get_tamable_data(&self) -> &TamableData {
        &self.tamable_data
    }
}

impl NeutralMob for WolfEntity {
    fn get_neutral_data(&self) -> &NeutralData {
        &self.neutral_data
    }
}

/// Vanilla `Wolf.BABY_DIMENSIONS`.
const BABY_DIMENSIONS: EntityDimensions = pumpkin_data::entity_dimensions::WOLF_BABY_DIMENSIONS;

impl Mob for WolfEntity {
    /// Vanilla `Wolf.canBeLeashed`.
    fn can_be_leashed(&self) -> bool {
        !self.is_angry()
    }

    /// Vanilla `Wolf.actuallyHurt`: wolf armour takes the whole hit as durability, and cracks
    /// audibly as it wears.
    fn absorb_hurt(
        &self,
        caller: &dyn crate::entity::EntityBase,
        damage_type: pumpkin_data::damage::DamageType,
        damage: f32,
    ) -> bool {
        use pumpkin_data::data_component_impl::EquipmentSlot;
        let living = &self.get_mob_entity().living_entity;
        let armor = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::BODY);
        // `canArmorAbsorb`.
        if armor.item != &pumpkin_data::item::Item::WOLF_ARMOR
            || damage_type.has_tag(&pumpkin_data::tag::DamageType::MINECRAFT_BYPASSES_WOLF_ARMOR)
        {
            return false;
        }
        let crackiness_before = wolf_armor_crackiness(&armor);
        living.hurt_and_break_in_slot(caller, &EquipmentSlot::BODY, damage.ceil() as i32);
        let armor_after = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::BODY);
        if crackiness_before != wolf_armor_crackiness(&armor_after) {
            caller.play_sound_at_self(pumpkin_data::sound::Sound::ItemWolfArmorCrack, 1.0, 1.0);
            let scute = pumpkin_data::item_stack::ItemStack::new(
                1,
                &pumpkin_data::item::Item::ARMADILLO_SCUTE,
            );
            let mut data = Vec::new();
            if pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer(
                std::borrow::Cow::Owned(scute),
            )
            .write_template0(
                &mut data,
                &pumpkin_util::version::JavaMinecraftVersion::V_26_1,
            )
            .is_ok()
            {
                let pos = living.entity.pos.load();
                living.entity.world.load().spawn_particle_with_data(
                    pumpkin_util::math::vector3::Vector3::new(pos.x, pos.y + 1.0, pos.z),
                    pumpkin_util::math::vector3::Vector3::new(0.2, 0.1, 0.2),
                    0.1,
                    20,
                    pumpkin_data::particle::Particle::Item,
                    &data,
                );
            }
        }
        true
    }

    /// Vanilla `Wolf.hurtArmor`.
    fn hurt_armor(&self, damage_type: pumpkin_data::damage::DamageType, damage: f32) {
        self.get_mob_entity().living_entity.do_hurt_equipment(
            self,
            damage_type,
            damage,
            &[pumpkin_data::data_component_impl::EquipmentSlot::BODY],
        );
    }

    /// Vanilla `Wolf.playStepSound`.
    fn play_step_sound(
        &self,
        caller: &dyn crate::entity::EntityBase,
        _pos: &pumpkin_util::math::position::BlockPos,
        _block: &'static pumpkin_data::Block,
        _state: &'static pumpkin_data::BlockState,
    ) {
        let sound = WolfSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
            .unwrap_or_default()
            .step_sound(self.get_entity().is_baby());
        caller.play_sound_at_self(sound, 0.15, 1.0);
    }

    /// Vanilla `Wolf.getAmbientSound`.
    fn ambient_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        let sounds = WolfSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
            .unwrap_or_default();
        let baby = self.get_entity().is_baby();
        Some(
            if self
                .as_neutral()
                .is_some_and(crate::entity::mob::neutral::NeutralMob::is_angry)
            {
                sounds.growl_sound(baby)
            } else if self.get_entity().random().next_bounded_i32(3) == 0 {
                if self.is_tame() && self.get_mob_entity().living_entity.health.load() < 20.0 {
                    sounds.whine_sound(baby)
                } else {
                    sounds.pant_sound(baby)
                }
            } else {
                sounds.ambient_sound(baby)
            },
        )
    }

    /// Vanilla `Wolf.getDefaultDimensions`.
    fn default_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        let entity = self.get_entity();
        if entity.is_baby() {
            BABY_DIMENSIONS
        } else {
            entity.get_dimensions(pose)
        }
    }

    fn as_ageable(&self) -> Option<&dyn AgeableMob> {
        Some(self)
    }

    /// Vanilla `AgeableMob.aiStep`, which runs inside this mob's `super.aiStep()`.
    fn ai_step_end(&self, _caller: &dyn EntityBase) {
        self.ageable_ai_step();
    }

    fn as_animal(&self) -> Option<&dyn Animal> {
        Some(self)
    }

    fn as_tamable(&self) -> Option<&dyn TamableAnimal> {
        Some(self)
    }

    fn as_neutral(&self) -> Option<&dyn NeutralMob> {
        Some(self)
    }

    fn can_attack_with_owner(&self, target: &dyn EntityBase, owner: &dyn EntityBase) -> bool {
        let target_entity = target.get_entity();
        let target_type = target_entity.entity_type;
        if *target_type == EntityType::CREEPER
            || *target_type == EntityType::GHAST
            || *target_type == EntityType::ARMOR_STAND
        {
            return false;
        }

        if *target_type == EntityType::WOLF {
            if let Some(target_mob) = target.get_mob()
                && let Some(tamable) = target_mob.as_tamable()
                && tamable.is_tame()
                && let Some(target_owner) = tamable.get_owner()
                && let Some(owner_player) = owner.get_player()
                && target_owner == owner_player.gameprofile.id
            {
                return false;
            }
            return true;
        }

        if *target_type == EntityType::PLAYER {
            if let Some(owner_player) = owner.get_player()
                && let Some(target_player) = target.get_player()
            {
                if owner_player.gameprofile.id == target_player.gameprofile.id {
                    return false;
                }
                let world = target_player.world();
                if !world.level_info.load().game_rules.pvp {
                    return false;
                }
            }
            return true;
        }

        if let Some(target_mob) = target.get_mob()
            && let Some(tamable) = target_mob.as_tamable()
            && tamable.is_tame()
        {
            return false;
        }

        true
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        let variant_id = self.variant.load(Ordering::Relaxed);
        let variant_str = WolfVariant::all()
            .get(variant_id as usize)
            .map_or("minecraft:pale", WolfVariant::asset_id);
        nbt.put_string("variant", variant_str.to_string());
        let sound_variant = WolfSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
            .unwrap_or_default();
        nbt.put_string(
            "sound_variant",
            format!("minecraft:{}", sound_variant.to_name()),
        );
        nbt.put_byte(
            "CollarColor",
            self.collar_color.load(Ordering::Relaxed) as i8,
        );
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        if let Some(variant_str) = nbt.get_string("variant") {
            let variant =
                WolfVariant::from_name(variant_str).map_or(WolfVariant::Pale.id(), |v| v.id());
            self.variant.store(variant, Ordering::Relaxed);
        }
        if let Some(sound_str) = nbt.get_string("sound_variant")
            && let Some(sound_variant) = WolfSoundVariant::from_name(sound_str)
        {
            self.sound_variant
                .store(sound_variant as u8, Ordering::Relaxed);
        }
        if let Some(collar) = nbt.get_byte("CollarColor") {
            self.collar_color.store(collar as u8, Ordering::Relaxed);
        } else if let Some(collar_int) = nbt.get_int("CollarColor") {
            self.collar_color.store(collar_int as u8, Ordering::Relaxed);
        }
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn as_custom_sound(&self) -> Option<&dyn CustomSound> {
        Some(self)
    }

    fn mob_set_variant_name(&self, name: &str) {
        let variant = WolfVariant::from_name(name).map_or(WolfVariant::Pale.id(), |v| v.id());
        self.variant.store(variant, Ordering::Relaxed);
    }

    fn mob_set_sound_variant_name(&self, name: &str) {
        if let Some(v) = WolfSoundVariant::from_name(name) {
            self.set_sound_variant(v);
        }
    }

    fn mob_init_data_tracker(&self) {
        let entity = self.get_entity();
        let is_baby = entity.age.load(Ordering::Relaxed) < 0;
        if is_baby {
            entity.set_synced_data(pumpkin_data::tracked_data::wolf::BABY_ID, true);
        }
        entity.set_synced_data(
            pumpkin_data::tracked_data::wolf::TAMEABLE_FLAGS,
            self.get_tame_flags(),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::wolf::COLLAR_COLOR,
            VarInt(self.collar_color.load(Ordering::Relaxed) as i32),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::wolf::WOLF_VARIANT_ID,
            VarInt(self.variant.load(Ordering::Relaxed) as i32),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::wolf::DATA_SOUND_VARIANT_ID,
            VarInt(self.sound_variant.load(Ordering::Relaxed) as i32),
        );
        entity.set_synced_data(
            pumpkin_data::tracked_data::wolf::OWNER_UUID,
            self.get_owner(),
        );
    }

    fn mob_interact(&self, player: &Arc<Player>, item_stack: &mut ItemStack) -> bool {
        let item = item_stack.get_item();
        let sound_variant = WolfSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
            .unwrap_or_default();
        let ambient = sound_variant.ambient_sound(self.is_baby());
        if self.is_tame() {
            if self.is_food(item_stack)
                && self.mob_entity.living_entity.health.load()
                    < self.mob_entity.living_entity.get_max_health()
            {
                item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                self.mob_entity.living_entity.heal(2.0);
                self.play_eating_sound(ambient);
                return true;
            }

            if self.is_owned_by(&player.gameprofile.id) {
                if let Some(color) = super::animal::get_dye_color_from_item(item)
                    && color != self.get_collar_color()
                {
                    self.set_collar_color(color);
                    item_stack.decrement_unless_creative(player.gamemode.load(), 1);
                    return true;
                }

                let parent_interaction = self.animal_interact(player, item_stack, ambient);
                if !parent_interaction {
                    self.set_ordered_to_sit(!self.is_ordered_to_sit());
                    return true;
                }
                return parent_interaction;
            }
        } else if item == &Item::BONE && !self.mob_entity.is_attacking() {
            item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            // `Wolf.tryToTame`: `this.random.nextInt(3) == 0`.
            if self.get_entity().random().next_bounded_i32(3) == 0 {
                TamableAnimal::tame(self, player.gameprofile.id);
                self.set_ordered_to_sit(true);
                self.spawn_taming_particles(true);
            } else {
                self.spawn_taming_particles(false);
            }
            return true;
        }

        self.animal_interact(player, item_stack, ambient)
    }
}

impl CustomSound for WolfEntity {
    fn death_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        let is_baby = self.is_baby();
        let sound_variant = WolfSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
            .unwrap_or_default();
        Some(sound_variant.death_sound(is_baby))
    }

    fn hurt_sound(&self) -> Option<pumpkin_data::sound::Sound> {
        let is_baby = self.is_baby();
        let sound_variant = WolfSoundVariant::from_id(self.sound_variant.load(Ordering::Relaxed))
            .unwrap_or_default();
        Some(sound_variant.hurt_sound(is_baby))
    }
}

impl WolfEntity {
    pub fn get_collar_color(&self) -> u8 {
        self.collar_color.load(Ordering::Relaxed)
    }

    pub fn set_collar_color(&self, color: u8) {
        self.collar_color.store(color, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::wolf::COLLAR_COLOR,
            VarInt(color as i32),
        );
    }

    pub fn set_variant(&self, variant: WolfVariant) {
        self.variant.store(variant.id(), Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::wolf::WOLF_VARIANT_ID,
            VarInt(variant.id() as i32),
        );
    }

    pub fn set_sound_variant(&self, sound_variant: WolfSoundVariant) {
        self.sound_variant
            .store(sound_variant as u8, Ordering::Relaxed);
        let entity = self.get_entity();
        entity.set_synced_data(
            pumpkin_data::tracked_data::wolf::DATA_SOUND_VARIANT_ID,
            VarInt(sound_variant as u8 as i32),
        );
    }
}

/// Vanilla `AbstractSkeleton` class membership.
fn is_abstract_skeleton(entity_type: &EntityType) -> bool {
    [
        &EntityType::SKELETON,
        &EntityType::STRAY,
        &EntityType::WITHER_SKELETON,
        &EntityType::BOGGED,
        &EntityType::PARCHED,
    ]
    .contains(&entity_type)
}

/// Vanilla `Crackiness.WOLF_ARMOR.byDamage`: 0 (none) to 3 (high) as the armour wears.
fn wolf_armor_crackiness(armor: &pumpkin_data::item_stack::ItemStack) -> u8 {
    if !armor.is_damageable() {
        return 0;
    }
    let max_damage = armor.get_max_damage().unwrap_or(0);
    let fraction = (max_damage - armor.get_damage()) as f32 / max_damage as f32;
    if fraction < 0.32 {
        3
    } else if fraction < 0.69 {
        2
    } else {
        u8::from(fraction < 0.95)
    }
}
