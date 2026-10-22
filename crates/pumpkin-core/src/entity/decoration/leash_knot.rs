use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase, leash, leashable, living::LivingEntity};
use crate::item::interaction_result::InteractionResult;
use crate::world::World;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag::Taggable;
use pumpkin_util::Hand;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

pub struct LeashKnotEntity {
    entity: Entity,
    pos: BlockPos,
    /// Vanilla `BlockAttachedEntity.ticksSinceLastCheck` (not saved).
    ticks_since_last_check: AtomicI32,
}

impl LeashKnotEntity {
    pub const OFFSET_Y: f64 = 0.375;
    /// Vanilla `BlockAttachedEntity.CHECK_INTERVAL`.
    const CHECK_INTERVAL: i32 = 100;

    pub const fn new(entity: Entity, pos: BlockPos) -> Self {
        Self {
            entity,
            pos,
            ticks_since_last_check: AtomicI32::new(0),
        }
    }

    pub const fn block_pos(&self) -> BlockPos {
        self.pos
    }

    /// Vanilla `LeashFenceKnotEntity.getOrCreateKnot`; `None` when a plugin cancels the new
    /// knot's spawn.
    pub fn get_or_create(world: &Arc<World>, pos: BlockPos) -> Option<Arc<dyn EntityBase>> {
        Self::get_knot(world, pos).or_else(|| Self::create_knot(world, pos))
    }

    /// Vanilla `LeashFenceKnotEntity.getKnot`: the knots touching the box `pos ± 1` whose block is
    /// `pos`.
    pub fn get_knot(world: &Arc<World>, pos: BlockPos) -> Option<Arc<dyn EntityBase>> {
        let x = f64::from(pos.0.x);
        let y = f64::from(pos.0.y);
        let z = f64::from(pos.0.z);
        let search_box = BoundingBox::new(
            Vector3::new(x - 1.0, y - 1.0, z - 1.0),
            Vector3::new(x + 1.0, y + 1.0, z + 1.0),
        );
        world
            .get_entities_at_box(&search_box)
            .into_iter()
            .find(|entity_base| {
                entity_base
                    .cast_any()
                    .downcast_ref::<Self>()
                    .is_some_and(|knot| knot.pos == pos)
            })
    }

    /// Vanilla `LeashFenceKnotEntity.createKnot`: `addFreshEntity`, silently. `None` when a plugin
    /// cancels the spawn (vanilla has no cancel), so nothing is tied to a knot outside the world.
    pub fn create_knot(world: &Arc<World>, pos: BlockPos) -> Option<Arc<dyn EntityBase>> {
        let raw_pos = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + Self::OFFSET_Y,
            f64::from(pos.0.z) + 0.5,
        );

        let entity = Entity::new(world.clone(), raw_pos, &EntityType::LEASH_KNOT);
        let knot: Arc<dyn EntityBase> = Arc::new(Self::new(entity, pos));
        world.spawn_entity(knot.clone()).then_some(knot)
    }

    /// Vanilla `LeashFenceKnotEntity.playPlacementSound`.
    pub fn play_placement_sound(&self) {
        self.play_sound_at_self(Sound::ItemLeadTied, 1.0, 1.0);
    }

    /// Vanilla `LeashFenceKnotEntity.survives`: the block is still a fence.
    fn survives(&self) -> bool {
        self.entity
            .world
            .load()
            .get_block(&self.pos)
            .has_tag(&pumpkin_data::tag::Block::MINECRAFT_FENCES)
    }

    /// Vanilla `LeashFenceKnotEntity.dropItem`: only the untie sound, no item.
    fn drop_item(&self) {
        self.play_sound_at_self(Sound::ItemLeadUntied, 1.0, 1.0);
    }
}

use crate::server::Server;

impl EntityBase for LeashKnotEntity {
    /// Vanilla `LeashFenceKnotEntity.notifyLeasheeRemoved`: a knot that holds nothing more is
    /// discarded.
    fn notify_leashee_removed(&self, _caller: &dyn EntityBase, _leashable: &dyn EntityBase) {
        if leashable::leashable_leashed_to(&self.entity).is_empty() {
            self.entity.remove();
        }
    }

    /// Vanilla `BlockAttachedEntity.skipAttackInteraction`.
    fn skip_attack_interaction(&self, source: &dyn EntityBase) -> bool {
        super::block_attached_skip_attack_interaction(self, source)
    }

    /// Vanilla `BlockAttachedEntity.isPickable`.
    fn can_hit(&self) -> bool {
        true
    }

    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    /// Vanilla `BlockAttachedEntity.tick`: below the world → discarded; every 100 ticks a knot
    /// whose fence is gone is discarded and `dropItem`s. Its leashees drop their own leads in
    /// their next `tickLeash`, as the holder can't interact with the level any more.
    fn tick(&self, caller: &dyn EntityBase, _server: &Server) {
        self.entity.check_out_of_world(caller);
        if self.ticks_since_last_check.fetch_add(1, Ordering::Relaxed) >= Self::CHECK_INTERVAL {
            self.ticks_since_last_check.store(0, Ordering::Relaxed);
            if !self.entity.is_removed() && !self.survives() {
                self.entity.remove();
                self.drop_item();
            }
        }
    }

    /// Vanilla `LeashFenceKnotEntity.interact`: shears cut what the knot holds; else the
    /// player's leashed mobs are tied to the knot, or the knot's mobs go to a player who isn't
    /// sneaking.
    fn interact(
        &self,
        player: &Arc<Player>,
        hand: Hand,
        item_stack: &mut ItemStack,
    ) -> InteractionResult {
        let world = self.entity.world.load_full();
        let Some(this) = world.get_entity_by_id(self.entity.entity_id) else {
            return InteractionResult::PASS;
        };
        if item_stack.item == &pumpkin_data::item::Item::SHEARS {
            let result = leash::entity_interact(&this, player, hand, item_stack);
            // `wasItemInteraction`: on a knot, only the shears' cut succeeds in
            // `Entity.interact`, and that is an item interaction.
            if result.consumes_action() {
                return result;
            }
        }

        let mut attached_mob = false;
        for leashee in leashable::leashable_leashed_to(player.get_entity()) {
            if leashable::can_have_a_leash_attached_to(leashee.as_ref(), &self.entity) {
                leashable::set_leashed_to(leashee.as_ref(), this.clone(), true);
                attached_mob = true;
            }
        }

        let mut any_dropped = false;
        if !attached_mob && !player.get_entity().is_sneaking() {
            for mob in leashable::leashable_leashed_to(&self.entity) {
                if leashable::can_have_a_leash_attached_to(mob.as_ref(), player.get_entity()) {
                    leashable::set_leashed_to(mob.as_ref(), player.clone(), true);
                    any_dropped = true;
                }
            }
        }

        if !attached_mob && !any_dropped {
            return leash::entity_interact(&this, player, hand, item_stack);
        }

        let player_base: &dyn EntityBase = player.as_ref();
        world.game_event_from(
            Some(player_base),
            pumpkin_data::game_event::GameEvent::BlockAttach,
            self.entity.pos.load(),
        );
        self.play_sound_at_self(Sound::ItemLeadTied, 1.0, 1.0);
        InteractionResult::SUCCESS
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
