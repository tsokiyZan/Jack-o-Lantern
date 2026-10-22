use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase, leash, living::LivingEntity};
use crate::item::interaction_result::InteractionResult;
use crate::world::World;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_util::Hand;
use pumpkin_util::math::boundingbox::{BoundingBox, EntityDimensions};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

pub struct LeashKnotEntity {
    entity: Entity,
    pos: BlockPos,
}

impl LeashKnotEntity {
    pub const OFFSET_Y: f64 = 0.375;

    pub const fn new(entity: Entity, pos: BlockPos) -> Self {
        Self { entity, pos }
    }

    pub const fn block_pos(&self) -> BlockPos {
        self.pos
    }

    pub fn get_or_create(world: &Arc<World>, pos: BlockPos) -> Arc<Self> {
        if let Some(existing) = Self::get_knot(world, pos) {
            return existing;
        }
        Self::create_knot(world, pos)
    }

    pub fn get_knot(world: &Arc<World>, pos: BlockPos) -> Option<Arc<Self>> {
        let center = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + Self::OFFSET_Y,
            f64::from(pos.0.z) + 0.5,
        );

        let search_dim = EntityDimensions::new(2.0, 2.0, 1.0);

        let search_box = BoundingBox::new_from_pos(center.x, center.y, center.z, &search_dim);
        let entities = world.get_entities_at_box(&search_box);

        for entity_base in entities {
            if entity_base.get_entity().entity_type == &EntityType::LEASH_KNOT
                && let Some(knot) = entity_base.cast_any().downcast_ref::<Arc<Self>>()
                && knot.pos == pos
            {
                return Some(knot.clone());
            }
        }
        None
    }

    pub fn create_knot(world: &Arc<World>, pos: BlockPos) -> Arc<Self> {
        let raw_pos = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + Self::OFFSET_Y,
            f64::from(pos.0.z) + 0.5,
        );

        let entity = Entity::new(world.clone(), raw_pos, &EntityType::LEASH_KNOT);
        let knot = Arc::new(Self::new(entity, pos));
        world.spawn_entity(knot.clone() as Arc<dyn EntityBase>);

        world.play_sound(Sound::ItemLeadTied, SoundCategory::Neutral, &raw_pos);

        knot
    }

    pub fn play_placement_sound(&self, world: &World) {
        let pos = self.entity.pos.load();
        world.play_sound(Sound::ItemLeadTied, SoundCategory::Neutral, &pos);
    }
}

use crate::server::Server;

impl EntityBase for LeashKnotEntity {
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

    fn tick(&self, _caller: &dyn EntityBase, _server: &Server) {
        let world = self.entity.world.load();
        let block = world.get_block(&self.pos);
        if !block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_FENCES) {
            let knot_id = self.entity.entity_id;
            let search_dim = EntityDimensions::new(32.0, 32.0, 16.0);
            let pos = self.entity.pos.load();
            let search_box = BoundingBox::new_from_pos(pos.x, pos.y, pos.z, &search_dim);
            let entities = world.get_entities_at_box(&search_box);

            for entity_base in entities {
                let ent = entity_base.get_entity();
                let is_attached_to_knot = ent
                    .leashed_to
                    .try_lock()
                    .ok()
                    .and_then(|guard| {
                        guard
                            .as_ref()
                            .map(|holder| holder.get_entity().entity_id == knot_id)
                    })
                    .unwrap_or(false);

                if is_attached_to_knot {
                    ent.unleash();
                    let lead_item = pumpkin_data::item_stack::ItemStack::new(
                        1,
                        &pumpkin_data::item::Item::LEAD,
                    );
                    world.drop_stack(&ent.block_pos.load(), lead_item);
                }
            }

            world.play_sound(Sound::ItemLeadUntied, SoundCategory::Neutral, &pos);
            self.entity.remove();
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
        for leashable in leash::leashable_leashed_to(&world, player.get_entity()) {
            if leash::can_have_a_leash_attached_to(leashable.as_ref(), &self.entity) {
                leash::set_leashed_to(leashable.as_ref(), this.clone());
                attached_mob = true;
            }
        }

        let mut any_dropped = false;
        if !attached_mob && !player.get_entity().is_sneaking() {
            for mob in leash::leashable_leashed_to(&world, &self.entity) {
                if leash::can_have_a_leash_attached_to(mob.as_ref(), player.get_entity()) {
                    leash::set_leashed_to(mob.as_ref(), player.clone());
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
