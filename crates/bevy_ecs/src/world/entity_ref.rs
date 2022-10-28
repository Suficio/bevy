use crate::{
    archetype::{Archetype, ArchetypeId, Archetypes},
    bundle::{Bundle, BundleInfo, DynamicBundle},
    change_detection::{MutUntyped, Ticks},
    component::{Component, ComponentId, ComponentTicks, Components, StorageType},
    entity::{Entities, Entity, EntityLocation},
    storage::{SparseSet, Storages},
    world::{Mut, World},
};
use bevy_ptr::{OwningPtr, Ptr};
use bevy_utils::tracing::debug;
use std::any::TypeId;

/// A read-only reference to a particular [`Entity`] and all of its components
#[derive(Copy, Clone)]
pub struct EntityRef<'w> {
    world: &'w World,
    entity: Entity,
    location: EntityLocation,
}

impl<'w> EntityRef<'w> {
    #[inline]
    pub(crate) fn new(world: &'w World, entity: Entity, location: EntityLocation) -> Self {
        Self {
            world,
            entity,
            location,
        }
    }

    #[inline]
    #[must_use = "Omit the .id() call if you do not need to store the `Entity` identifier."]
    pub fn id(&self) -> Entity {
        self.entity
    }

    #[inline]
    pub fn location(&self) -> EntityLocation {
        self.location
    }

    #[inline]
    pub fn archetype(&self) -> &Archetype {
        &self.world.archetypes[self.location.archetype_id]
    }

    #[inline]
    pub fn world(&self) -> &'w World {
        self.world
    }

    #[inline]
    pub fn contains<T: Component>(&self) -> bool {
        self.contains_type_id(TypeId::of::<T>())
    }

    #[inline]
    pub fn contains_id(&self, component_id: ComponentId) -> bool {
        contains_component_with_id(self.world, component_id, self.location)
    }

    #[inline]
    pub fn contains_type_id(&self, type_id: TypeId) -> bool {
        contains_component_with_type(self.world, type_id, self.location)
    }

    #[inline]
    pub fn get<T: Component>(&self) -> Option<&'w T> {
        // SAFETY:
        // - entity location and entity is valid
        // - the storage type provided is correct for T
        // - world access is immutable, lifetime tied to `&self`
        unsafe {
            self.world
                .get_component_with_type(
                    TypeId::of::<T>(),
                    T::Storage::STORAGE_TYPE,
                    self.entity,
                    self.location,
                )
                // SAFETY: returned component is of type T
                .map(|value| value.deref::<T>())
        }
    }

    /// Retrieves the change ticks for the given component. This can be useful for implementing change
    /// detection in custom runtimes.
    #[inline]
    pub fn get_change_ticks<T: Component>(&self) -> Option<ComponentTicks> {
        // SAFETY:
        // - entity location and entity is valid
        // - world access is immutable, lifetime tied to `&self`
        // - the storage type provided is correct for T
        unsafe {
            self.world.get_ticks_with_type(
                TypeId::of::<T>(),
                T::Storage::STORAGE_TYPE,
                self.entity,
                self.location,
            )
        }
    }

    /// Retrieves the change ticks for the given [`ComponentId`]. This can be useful for implementing change
    /// detection in custom runtimes.
    ///
    /// **You should prefer to use the typed API [`EntityRef::get_change_ticks`] where possible and only
    /// use this in cases where the actual component types are not known at
    /// compile time.**
    #[inline]
    pub fn get_change_ticks_by_id(&self, component_id: ComponentId) -> Option<ComponentTicks> {
        let info = self.world.components().get_info(component_id)?;
        // SAFETY:
        // - entity location and entity is valid
        // - world access is immutable, lifetime tied to `&self`
        // - the storage type provided is correct for T
        unsafe {
            self.world.get_ticks(
                component_id,
                info.storage_type(),
                self.entity,
                self.location,
            )
        }
    }

    /// Gets a mutable reference to the component of type `T` associated with
    /// this entity without ensuring there are no other borrows active and without
    /// ensuring that the returned reference will stay valid.
    ///
    /// # Safety
    ///
    /// - The returned reference must never alias a mutable borrow of this component.
    /// - The returned reference must not be used after this component is moved which
    ///   may happen from **any** `insert_component`, `remove_component` or `despawn`
    ///   operation on this world (non-exhaustive list).
    #[inline]
    pub unsafe fn get_unchecked_mut<T: Component>(
        &self,
        last_change_tick: u32,
        change_tick: u32,
    ) -> Option<Mut<'w, T>> {
        // SAFETY:
        // - entity location and entity is valid
        // - returned component is of type T
        // - the storage type provided is correct for T
        self.world
            .get_component_and_ticks_with_type(
                TypeId::of::<T>(),
                T::Storage::STORAGE_TYPE,
                self.entity,
                self.location,
            )
            .map(|(value, ticks)| Mut {
                // SAFETY:
                // - returned component is of type T
                // - Caller guarantees that this reference will not alias.
                value: value.assert_unique().deref_mut::<T>(),
                ticks: TicksMut::from_tick_cells(ticks, last_change_tick, change_tick),
            })
    }
}

impl<'w> EntityRef<'w> {
    /// Gets the component of the given [`ComponentId`] from the entity.
    ///
    /// **You should prefer to use the typed API where possible and only
    /// use this in cases where the actual component types are not known at
    /// compile time.**
    ///
    /// Unlike [`EntityRef::get`], this returns a raw pointer to the component,
    /// which is only valid while the `'w` borrow of the lifetime is active.
    #[inline]
    pub fn get_by_id(&self, component_id: ComponentId) -> Option<Ptr<'w>> {
        let info = self.world.components().get_info(component_id)?;
        // SAFETY:
        // - entity_location and entity are valid
        // . component_id is valid as checked by the line above
        // - the storage type is accurate as checked by the fetched ComponentInfo
        unsafe {
            self.world.get_component(
                component_id,
                info.storage_type(),
                self.entity,
                self.location,
            )
        }
    }
}

impl<'w> From<EntityMut<'w>> for EntityRef<'w> {
    fn from(entity_mut: EntityMut<'w>) -> EntityRef<'w> {
        EntityRef::new(entity_mut.world, entity_mut.entity, entity_mut.location)
    }
}

/// A mutable reference to a particular [`Entity`] and all of its components
pub struct EntityMut<'w> {
    world: &'w mut World,
    entity: Entity,
    location: EntityLocation,
}

impl<'w> EntityMut<'w> {
    /// # Safety
    /// entity and location _must_ be valid
    #[inline]
    pub(crate) unsafe fn new(
        world: &'w mut World,
        entity: Entity,
        location: EntityLocation,
    ) -> Self {
        EntityMut {
            world,
            entity,
            location,
        }
    }

    #[inline]
    #[must_use = "Omit the .id() call if you do not need to store the `Entity` identifier."]
    pub fn id(&self) -> Entity {
        self.entity
    }

    #[inline]
    pub fn location(&self) -> EntityLocation {
        self.location
    }

    #[inline]
    pub fn archetype(&self) -> &Archetype {
        &self.world.archetypes[self.location.archetype_id]
    }

    #[inline]
    pub fn contains<T: Component>(&self) -> bool {
        self.contains_type_id(TypeId::of::<T>())
    }

    #[inline]
    pub fn contains_id(&self, component_id: ComponentId) -> bool {
        contains_component_with_id(self.world, component_id, self.location)
    }

    #[inline]
    pub fn contains_type_id(&self, type_id: TypeId) -> bool {
        contains_component_with_type(self.world, type_id, self.location)
    }

    #[inline]
    pub fn get<T: Component>(&self) -> Option<&'_ T> {
        // SAFETY:
        // - entity location is valid
        // - world access is immutable, lifetime tied to `&self`
        // - the storage type provided is correct for T
        unsafe {
            self.world
                .get_component_with_type(
                    TypeId::of::<T>(),
                    T::Storage::STORAGE_TYPE,
                    self.entity,
                    self.location,
                )
                // SAFETY: returned component is of type T
                .map(|value| value.deref::<T>())
        }
    }

    #[inline]
    pub fn get_mut<T: Component>(&mut self) -> Option<Mut<'_, T>> {
        // SAFETY: world access is unique, and lifetimes enforce correct usage of returned borrow
        unsafe { self.get_unchecked_mut::<T>() }
    }

    /// Retrieves the change ticks for the given component. This can be useful for implementing change
    /// detection in custom runtimes.
    #[inline]
    pub fn get_change_ticks<T: Component>(&self) -> Option<ComponentTicks> {
        // SAFETY:
        // - entity location is valid
        // - world access is immutable, lifetime tied to `&self`
        // - the storage type provided is correct for T
        unsafe {
            self.world.get_ticks_with_type(
                TypeId::of::<T>(),
                T::Storage::STORAGE_TYPE,
                self.entity,
                self.location,
            )
        }
    }

    /// Retrieves the change ticks for the given [`ComponentId`]. This can be useful for implementing change
    /// detection in custom runtimes.
    ///
    /// **You should prefer to use the typed API [`EntityMut::get_change_ticks`] where possible and only
    /// use this in cases where the actual component types are not known at
    /// compile time.**
    #[inline]
    pub fn get_change_ticks_by_id(&self, component_id: ComponentId) -> Option<ComponentTicks> {
        let info = self.world.components().get_info(component_id)?;
        // SAFETY:
        // - entity location is valid
        // - world access is immutable, lifetime tied to `&self`
        // - the storage type provided is correct for T
        unsafe {
            self.world.get_ticks(
                component_id,
                info.storage_type(),
                self.entity,
                self.location,
            )
        }
    }

    /// Gets a mutable reference to the component of type `T` associated with
    /// this entity without ensuring there are no other borrows active and without
    /// ensuring that the returned reference will stay valid.
    ///
    /// # Safety
    ///
    /// - The returned reference must never alias a mutable borrow of this component.
    /// - The returned reference must not be used after this component is moved which
    ///   may happen from **any** `insert_component`, `remove_component` or `despawn`
    ///   operation on this world (non-exhaustive list).
    #[inline]
    pub unsafe fn get_unchecked_mut<T: Component>(&self) -> Option<Mut<'_, T>> {
        // SAFETY:
        // - entity location and entity is valid
        // - returned component is of type T
        // - the storage type provided is correct for T
        self.world
            .get_component_and_ticks_with_type(
                TypeId::of::<T>(),
                T::Storage::STORAGE_TYPE,
                self.entity,
                self.location,
            )
            .map(|(value, ticks)| Mut {
                value: value.assert_unique().deref_mut::<T>(),
                ticks: TicksMut::from_tick_cells(
                    ticks,
                    self.world.last_change_tick(),
                    self.world.read_change_tick(),
                ),
            })
    }

    /// Adds a [`Bundle`] of components to the entity.
    ///
    /// This will overwrite any previous value(s) of the same component type.
    pub fn insert<T: Bundle>(&mut self, bundle: T) -> &mut Self {
        let change_tick = self.world.change_tick();
        let bundle_info = self
            .world
            .bundles
            .init_info::<T>(&mut self.world.components, &mut self.world.storages);
        let mut bundle_inserter = bundle_info.get_bundle_inserter(
            &mut self.world.entities,
            &mut self.world.archetypes,
            &mut self.world.components,
            &mut self.world.storages,
            self.location.archetype_id,
            change_tick,
        );
        // SAFETY: location matches current entity. `T` matches `bundle_info`
        unsafe {
            self.location = bundle_inserter.insert(self.entity, self.location, bundle);
        }

        self
    }

    /// Inserts a component with the given `value`. Will replace the value if it already existed.
    ///
    /// **You should prefer to use the typed API [`EntityMut::insert`] where possible and only
    /// use this in cases where there isn't a Rust type corresponding to the [`ComponentId`].
    ///
    /// # Safety
    /// The value referenced by `value` must be valid for the given [`ComponentId`] of this world
    pub unsafe fn insert_by_id(
        &mut self,
        component_id: ComponentId,
        value: OwningPtr<'_>,
    ) -> &mut Self {
        // SAFETY: the caller promisees that `value` is valid for the `component_id`
        self.insert_bundle_by_ids(vec![component_id], std::iter::once(value))
    }

    /// Inserts a bundle of components into the entity. Will replace the values if they already existed.
    ///
    /// **You should prefer to use the typed API [`EntityMut::insert_bundle`] where possible and only
    /// use this in cases where there are no Rust types corresponding to the [`ComponentId`]s.
    ///
    /// # Safety
    /// - each value of `components` must be valid for the [`ComponentId`] at the matching position in `component_ids` in this world
    pub unsafe fn insert_bundle_by_ids<'a, I: IntoIterator<Item = OwningPtr<'a>>>(
        &mut self,
        mut component_ids: Vec<ComponentId>,
        components: I,
    ) -> &mut Self {
        component_ids.sort();

        for &id in &component_ids {
            self.world.components().get_info(id).unwrap_or_else(|| {
                panic!(
                    "insert_bundle_by_ids called with component id {id:?} which doesn't exist in this world"
                )
            });
        }

        struct DynamicInsertBundle<'a, I: Iterator<Item = OwningPtr<'a>>> {
            components: I,
        }
        impl<'a, I: Iterator<Item = OwningPtr<'a>>> DynamicBundle for DynamicInsertBundle<'a, I> {
            fn get_components(self, func: &mut impl FnMut(OwningPtr<'_>)) {
                self.components.for_each(func);
            }
        }

        let bundle = DynamicInsertBundle {
            components: components.into_iter(),
        };

        let change_tick = self.world.change_tick();
        // SAFETY: component_ids are all valid, because they are checked in this function
        let bundle_info = self
            .world
            .bundles
            .init_info_dynamic(&mut self.world.components, component_ids);
        let mut bundle_inserter = bundle_info.get_bundle_inserter(
            &mut self.world.entities,
            &mut self.world.archetypes,
            &mut self.world.components,
            &mut self.world.storages,
            self.location.archetype_id,
            change_tick,
        );
        // SAFETY: location matches current entity. The `bundle` matches `bundle_info` components as promised by the caller.
        self.location = bundle_inserter.insert(self.entity, self.location.index, bundle);

        self
    }

    #[deprecated(
        since = "0.9.0",
        note = "Use `remove` instead, which now accepts bundles, components, and tuples of bundles and components."
    )]
    pub fn remove_bundle<T: Bundle>(&mut self) -> Option<T> {
        self.remove::<T>()
    }

    // TODO: move to BundleInfo
    /// Removes a [`Bundle`] of components from the entity and returns the bundle.
    ///
    /// Returns `None` if the entity does not contain the bundle.
    pub fn remove<T: Bundle>(&mut self) -> Option<T> {
        let archetypes = &mut self.world.archetypes;
        let storages = &mut self.world.storages;
        let components = &mut self.world.components;
        let entities = &mut self.world.entities;
        let removed_components = &mut self.world.removed_components;

        let bundle_info = self.world.bundles.init_info::<T>(components, storages);
        let old_location = self.location;
        // SAFETY: `archetype_id` exists because it is referenced in the old `EntityLocation` which is valid,
        // components exist in `bundle_info` because `Bundles::init_info` initializes a `BundleInfo` containing all components of the bundle type `T`
        let new_archetype_id = unsafe {
            remove_bundle_from_archetype(
                archetypes,
                storages,
                components,
                old_location.archetype_id,
                bundle_info,
                false,
            )?
        };

        if new_archetype_id == old_location.archetype_id {
            return None;
        }

        let mut bundle_components = bundle_info.component_ids.iter().cloned();
        let entity = self.entity;
        // SAFETY: bundle components are iterated in order, which guarantees that the component type
        // matches
        let result = unsafe {
            T::from_components(storages, &mut |storages| {
                let component_id = bundle_components.next().unwrap();
                // SAFETY:
                // - entity location is valid
                // - table row is removed below, without dropping the contents
                // - `components` comes from the same world as `storages`
                take_component(
                    storages,
                    components,
                    removed_components,
                    component_id,
                    entity,
                    old_location,
                )
            })
        };

        #[allow(clippy::undocumented_unsafe_blocks)] // TODO: document why this is safe
        unsafe {
            Self::move_entity_from_remove::<false>(
                entity,
                &mut self.location,
                old_location.archetype_id,
                old_location,
                entities,
                archetypes,
                storages,
                new_archetype_id,
            );
        }

        Some(result)
    }

    /// Safety: `new_archetype_id` must have the same or a subset of the components
    /// in `old_archetype_id`. Probably more safety stuff too, audit a call to
    /// this fn as if the code here was written inline
    ///
    /// when DROP is true removed components will be dropped otherwise they will be forgotten
    ///
    // We use a const generic here so that we are less reliant on
    // inlining for rustc to optimize out the `match DROP`
    #[allow(clippy::too_many_arguments)]
    unsafe fn move_entity_from_remove<const DROP: bool>(
        entity: Entity,
        self_location: &mut EntityLocation,
        old_archetype_id: ArchetypeId,
        old_location: EntityLocation,
        entities: &mut Entities,
        archetypes: &mut Archetypes,
        storages: &mut Storages,
        new_archetype_id: ArchetypeId,
    ) {
        let old_archetype = &mut archetypes[old_archetype_id];
        let remove_result = old_archetype.swap_remove(old_location.archetype_row);
        if let Some(swapped_entity) = remove_result.swapped_entity {
            entities.set(swapped_entity.index(), old_location);
        }
        let old_table_row = remove_result.table_row;
        let old_table_id = old_archetype.table_id();
        let new_archetype = &mut archetypes[new_archetype_id];

        let new_location = if old_table_id == new_archetype.table_id() {
            new_archetype.allocate(entity, old_table_row)
        } else {
            let (old_table, new_table) = storages
                .tables
                .get_2_mut(old_table_id, new_archetype.table_id());

            // SAFETY: old_table_row exists
            let move_result = if DROP {
                old_table.move_to_and_drop_missing_unchecked(old_table_row, new_table)
            } else {
                old_table.move_to_and_forget_missing_unchecked(old_table_row, new_table)
            };

            // SAFETY: move_result.new_row is a valid position in new_archetype's table
            let new_location = new_archetype.allocate(entity, move_result.new_row);

            // if an entity was moved into this entity's table spot, update its table row
            if let Some(swapped_entity) = move_result.swapped_entity {
                let swapped_location = entities.get(swapped_entity).unwrap();
                archetypes[swapped_location.archetype_id]
                    .set_entity_table_row(swapped_location.archetype_row, old_table_row);
            }

            new_location
        };

        *self_location = new_location;
        // SAFETY: The entity is valid and has been moved to the new location already.
        entities.set(entity.index(), new_location);
    }

    // TODO: move to BundleInfo
    /// Remove any components in the bundle that the entity has.
    pub fn remove_intersection<T: Bundle>(&mut self) {
        let archetypes = &mut self.world.archetypes;
        let storages = &mut self.world.storages;
        let components = &mut self.world.components;
        let entities = &mut self.world.entities;
        let removed_components = &mut self.world.removed_components;

        let bundle_info = self.world.bundles.init_info::<T>(components, storages);
        let old_location = self.location;

        // SAFETY: `archetype_id` exists because it is referenced in the old `EntityLocation` which is valid,
        // components exist in `bundle_info` because `Bundles::init_info` initializes a `BundleInfo` containing all components of the bundle type `T`
        let new_archetype_id = unsafe {
            remove_bundle_from_archetype(
                archetypes,
                storages,
                components,
                old_location.archetype_id,
                bundle_info,
                true,
            )
            .expect("intersections should always return a result")
        };

        if new_archetype_id == old_location.archetype_id {
            return;
        }

        let old_archetype = &mut archetypes[old_location.archetype_id];
        let entity = self.entity;
        for component_id in bundle_info.component_ids.iter().cloned() {
            if old_archetype.contains(component_id) {
                removed_components
                    .get_or_insert_with(component_id, Vec::new)
                    .push(entity);

                // Make sure to drop components stored in sparse sets.
                // Dense components are dropped later in `move_to_and_drop_missing_unchecked`.
                if let Some(StorageType::SparseSet) = old_archetype.get_storage_type(component_id) {
                    storages
                        .sparse_sets
                        .get_mut(component_id)
                        .unwrap()
                        .remove(entity);
                }
            }
        }

        #[allow(clippy::undocumented_unsafe_blocks)] // TODO: document why this is safe
        unsafe {
            Self::move_entity_from_remove::<true>(
                entity,
                &mut self.location,
                old_location.archetype_id,
                old_location,
                entities,
                archetypes,
                storages,
                new_archetype_id,
            );
        }
    }

    pub fn despawn(self) {
        debug!("Despawning entity {:?}", self.entity);
        let world = self.world;
        world.flush();
        let location = world
            .entities
            .free(self.entity)
            .expect("entity should exist at this point.");
        let table_row;
        let moved_entity;
        {
            let archetype = &mut world.archetypes[location.archetype_id];
            for component_id in archetype.components() {
                let removed_components = world
                    .removed_components
                    .get_or_insert_with(component_id, Vec::new);
                removed_components.push(self.entity);
            }
            let remove_result = archetype.swap_remove(location.archetype_row);
            if let Some(swapped_entity) = remove_result.swapped_entity {
                // SAFETY: swapped_entity is valid and the swapped entity's components are
                // moved to the new location immediately after.
                unsafe {
                    world.entities.set(swapped_entity.index(), location);
                }
            }
            table_row = remove_result.table_row;

            for component_id in archetype.sparse_set_components() {
                let sparse_set = world.storages.sparse_sets.get_mut(component_id).unwrap();
                sparse_set.remove(self.entity);
            }
            // SAFETY: table rows stored in archetypes always exist
            moved_entity = unsafe {
                world.storages.tables[archetype.table_id()].swap_remove_unchecked(table_row)
            };
        };

        if let Some(moved_entity) = moved_entity {
            let moved_location = world.entities.get(moved_entity).unwrap();
            world.archetypes[moved_location.archetype_id]
                .set_entity_table_row(moved_location.archetype_row, table_row);
        }
    }

    #[inline]
    pub fn world(&self) -> &World {
        self.world
    }

    /// Returns this `EntityMut`'s world.
    ///
    /// See [`EntityMut::world_scope`] or [`EntityMut::into_world_mut`] for a safe alternative.
    ///
    /// # Safety
    /// Caller must not modify the world in a way that changes the current entity's location
    /// If the caller _does_ do something that could change the location, `self.update_location()`
    /// must be called before using any other methods on this [`EntityMut`].
    #[inline]
    pub unsafe fn world_mut(&mut self) -> &mut World {
        self.world
    }

    /// Return this `EntityMut`'s [`World`], consuming itself.
    #[inline]
    pub fn into_world_mut(self) -> &'w mut World {
        self.world
    }

    /// Gives mutable access to this `EntityMut`'s [`World`] in a temporary scope.
    pub fn world_scope(&mut self, f: impl FnOnce(&mut World)) {
        f(self.world);
        self.update_location();
    }

    /// Updates the internal entity location to match the current location in the internal
    /// [`World`]. This is only needed if the user called [`EntityMut::world`], which enables the
    /// location to change.
    pub fn update_location(&mut self) {
        self.location = self.world.entities().get(self.entity).unwrap();
    }
}

impl<'w> EntityMut<'w> {
    /// Gets the component of the given [`ComponentId`] from the entity.
    ///
    /// **You should prefer to use the typed API [`EntityMut::get`] where possible and only
    /// use this in cases where the actual component types are not known at
    /// compile time.**
    ///
    /// Unlike [`EntityMut::get`], this returns a raw pointer to the component,
    /// which is only valid while the [`EntityMut`] is alive.
    #[inline]
    pub fn get_by_id(&self, component_id: ComponentId) -> Option<Ptr<'_>> {
        let info = self.world.components().get_info(component_id)?;
        // SAFETY:
        // - entity_location is valid
        // - component_id is valid as checked by the line above
        // - the storage type is accurate as checked by the fetched ComponentInfo
        unsafe {
            self.world.get_component(
                component_id,
                info.storage_type(),
                self.entity,
                self.location,
            )
        }
    }

    /// Gets a [`MutUntyped`] of the component of the given [`ComponentId`] from the entity.
    ///
    /// **You should prefer to use the typed API [`EntityMut::get_mut`] where possible and only
    /// use this in cases where the actual component types are not known at
    /// compile time.**
    ///
    /// Unlike [`EntityMut::get_mut`], this returns a raw pointer to the component,
    /// which is only valid while the [`EntityMut`] is alive.
    #[inline]
    pub fn get_mut_by_id(&mut self, component_id: ComponentId) -> Option<MutUntyped<'_>> {
        self.world.components().get_info(component_id)?;
        // SAFETY: entity_location is valid, component_id is valid as checked by the line above
        unsafe { get_mut_by_id(self.world, self.entity, self.location, component_id) }
    }
}

fn contains_component_with_type(world: &World, type_id: TypeId, location: EntityLocation) -> bool {
    if let Some(component_id) = world.components.get_id(type_id) {
        contains_component_with_id(world, component_id, location)
    } else {
        false
    }
}

fn contains_component_with_id(
    world: &World,
    component_id: ComponentId,
    location: EntityLocation,
) -> bool {
    world.archetypes[location.archetype_id].contains(component_id)
}

/// Removes a bundle from the given archetype and returns the resulting archetype (or None if the
/// removal was invalid). in the event that adding the given bundle does not result in an Archetype
/// change. Results are cached in the Archetype Graph to avoid redundant work.
/// if `intersection` is false, attempting to remove a bundle with components _not_ contained in the
/// current archetype will fail, returning None. if `intersection` is true, components in the bundle
/// but not in the current archetype will be ignored
///
/// # Safety
/// `archetype_id` must exist and components in `bundle_info` must exist
unsafe fn remove_bundle_from_archetype(
    archetypes: &mut Archetypes,
    storages: &mut Storages,
    components: &mut Components,
    archetype_id: ArchetypeId,
    bundle_info: &BundleInfo,
    intersection: bool,
) -> Option<ArchetypeId> {
    // check the archetype graph to see if the Bundle has been removed from this archetype in the
    // past
    let remove_bundle_result = {
        let current_archetype = &mut archetypes[archetype_id];
        if intersection {
            current_archetype
                .edges()
                .get_remove_bundle_intersection(bundle_info.id)
        } else {
            current_archetype.edges().get_remove_bundle(bundle_info.id)
        }
    };
    let result = if let Some(result) = remove_bundle_result {
        // this Bundle removal result is cached. just return that!
        result
    } else {
        let mut next_table_components;
        let mut next_sparse_set_components;
        let next_table_id;
        {
            let current_archetype = &mut archetypes[archetype_id];
            let mut removed_table_components = Vec::new();
            let mut removed_sparse_set_components = Vec::new();
            for component_id in bundle_info.component_ids.iter().cloned() {
                if current_archetype.contains(component_id) {
                    // SAFETY: bundle components were already initialized by bundles.get_info
                    let component_info = components.get_info_unchecked(component_id);
                    match component_info.storage_type() {
                        StorageType::Table => removed_table_components.push(component_id),
                        StorageType::SparseSet => removed_sparse_set_components.push(component_id),
                    }
                } else if !intersection {
                    // a component in the bundle was not present in the entity's archetype, so this
                    // removal is invalid cache the result in the archetype
                    // graph
                    current_archetype
                        .edges_mut()
                        .insert_remove_bundle(bundle_info.id, None);
                    return None;
                }
            }

            // sort removed components so we can do an efficient "sorted remove". archetype
            // components are already sorted
            removed_table_components.sort();
            removed_sparse_set_components.sort();
            next_table_components = current_archetype.table_components().collect();
            next_sparse_set_components = current_archetype.sparse_set_components().collect();
            sorted_remove(&mut next_table_components, &removed_table_components);
            sorted_remove(
                &mut next_sparse_set_components,
                &removed_sparse_set_components,
            );

            next_table_id = if removed_table_components.is_empty() {
                current_archetype.table_id()
            } else {
                // SAFETY: all components in next_table_components exist
                storages
                    .tables
                    .get_id_or_insert(&next_table_components, components)
            };
        }

        let new_archetype_id = archetypes.get_id_or_insert(
            next_table_id,
            next_table_components,
            next_sparse_set_components,
        );
        Some(new_archetype_id)
    };
    let current_archetype = &mut archetypes[archetype_id];
    // cache the result in an edge
    if intersection {
        current_archetype
            .edges_mut()
            .insert_remove_bundle_intersection(bundle_info.id, result);
    } else {
        current_archetype
            .edges_mut()
            .insert_remove_bundle(bundle_info.id, result);
    }
    result
}

fn sorted_remove<T: Eq + Ord + Copy>(source: &mut Vec<T>, remove: &[T]) {
    let mut remove_index = 0;
    source.retain(|value| {
        while remove_index < remove.len() && *value > remove[remove_index] {
            remove_index += 1;
        }

        if remove_index < remove.len() {
            *value != remove[remove_index]
        } else {
            true
        }
    });
}

// SAFETY: EntityLocation must be valid
#[inline]
pub(crate) unsafe fn get_mut<T: Component>(
    world: &mut World,
    entity: Entity,
    location: EntityLocation,
) -> Option<Mut<'_, T>> {
    let change_tick = world.change_tick();
    let last_change_tick = world.last_change_tick();
    // SAFETY:
    // - world access is unique
    // - entity location is valid
    // - and returned component is of type T
    world
        .get_component_and_ticks_with_type(
            TypeId::of::<T>(),
            T::Storage::STORAGE_TYPE,
            entity,
            location,
        )
        .map(|(value, ticks)| Mut {
            // SAFETY:
            // - world access is unique and ties world lifetime to `Mut` lifetime
            // - `value` is of type `T`
            value: value.assert_unique().deref_mut::<T>(),
            ticks: TicksMut::from_tick_cells(ticks, last_change_tick, change_tick),
        })
}

// SAFETY: EntityLocation must be valid, component_id must be valid
#[inline]
pub(crate) unsafe fn get_mut_by_id(
    world: &mut World,
    entity: Entity,
    location: EntityLocation,
    component_id: ComponentId,
) -> Option<MutUntyped<'_>> {
    let change_tick = world.change_tick();
    // SAFETY: component_id is valid
    let info = world.components.get_info_unchecked(component_id);
    // SAFETY:
    // - world access is unique
    // - entity location is valid
    // - and returned component is of type T
    world
        .get_component_and_ticks(component_id, info.storage_type(), entity, location)
        .map(|(value, ticks)| MutUntyped {
            // SAFETY: world access is unique and ties world lifetime to `MutUntyped` lifetime
            value: value.assert_unique(),
            ticks: TicksMut::from_tick_cells(ticks, world.last_change_tick(), change_tick),
        })
}

/// Moves component data out of storage.
///
/// This function leaves the underlying memory unchanged, but the component behind
/// returned pointer is semantically owned by the caller and will not be dropped in its original location.
/// Caller is responsible to drop component data behind returned pointer.
///
/// # Safety
/// - `location.table_row` must be in bounds of column of component id `component_id`
/// - `component_id` must be valid
/// - `components` must come from the same world as `self`
/// - The relevant table row **must be removed** by the caller once all components are taken, without dropping the value
#[inline]
pub(crate) unsafe fn take_component<'a>(
    storages: &'a mut Storages,
    components: &Components,
    removed_components: &mut SparseSet<ComponentId, Vec<Entity>>,
    component_id: ComponentId,
    entity: Entity,
    location: EntityLocation,
) -> OwningPtr<'a> {
    // SAFETY: caller promises component_id to be valid
    let component_info = components.get_info_unchecked(component_id);
    let removed_components = removed_components.get_or_insert_with(component_id, Vec::new);
    removed_components.push(entity);
    match component_info.storage_type() {
        StorageType::Table => {
            let table = &mut storages.tables[location.table_id];
            let components = table.get_column_mut(component_id).unwrap();
            // SAFETY:
            // - archetypes only store valid table_rows
            // - index is in bounds as promised by caller
            // - promote is safe because the caller promises to remove the table row without dropping it immediately afterwards
            components
                .get_data_unchecked_mut(location.table_row)
                .promote()
        }
        StorageType::SparseSet => storages
            .sparse_sets
            .get_mut(component_id)
            .unwrap()
            .remove_and_forget(entity)
            .unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use bevy_ptr::OwningPtr;

    use crate as bevy_ecs;
    use crate::component::ComponentId;
    use crate::prelude::*; // for the `#[derive(Component)]`

    #[test]
    fn sorted_remove() {
        let mut a = vec![1, 2, 3, 4, 5, 6, 7];
        let b = vec![1, 2, 3, 5, 7];
        super::sorted_remove(&mut a, &b);

        assert_eq!(a, vec![4, 6]);

        let mut a = vec![1];
        let b = vec![1];
        super::sorted_remove(&mut a, &b);

        assert_eq!(a, vec![]);

        let mut a = vec![1];
        let b = vec![2];
        super::sorted_remove(&mut a, &b);

        assert_eq!(a, vec![1]);
    }

    #[derive(Component)]
    struct TestComponent(u32);

    #[derive(Component)]
    struct TestComponent2(u32);

    #[test]
    fn entity_ref_get_by_id() {
        let mut world = World::new();
        let entity = world.spawn(TestComponent(42)).id();
        let component_id = world
            .components()
            .get_id(std::any::TypeId::of::<TestComponent>())
            .unwrap();

        let entity = world.entity(entity);
        let test_component = entity.get_by_id(component_id).unwrap();
        // SAFETY: points to a valid `TestComponent`
        let test_component = unsafe { test_component.deref::<TestComponent>() };

        assert_eq!(test_component.0, 42);
    }

    #[test]
    fn entity_mut_get_by_id() {
        let mut world = World::new();
        let entity = world.spawn(TestComponent(42)).id();
        let component_id = world
            .components()
            .get_id(std::any::TypeId::of::<TestComponent>())
            .unwrap();

        let mut entity_mut = world.entity_mut(entity);
        let mut test_component = entity_mut.get_mut_by_id(component_id).unwrap();
        {
            test_component.set_changed();
            let test_component =
                // SAFETY: `test_component` has unique access of the `EntityMut` and is not used afterwards
                unsafe { test_component.into_inner().deref_mut::<TestComponent>() };
            test_component.0 = 43;
        }

        let entity = world.entity(entity);
        let test_component = entity.get_by_id(component_id).unwrap();
        // SAFETY: `TestComponent` is the correct component type
        let test_component = unsafe { test_component.deref::<TestComponent>() };

        assert_eq!(test_component.0, 43);
    }

    #[test]
    fn entity_ref_get_by_id_invalid_component_id() {
        let invalid_component_id = ComponentId::new(usize::MAX);

        let mut world = World::new();
        let entity = world.spawn_empty().id();
        let entity = world.entity(entity);
        assert!(entity.get_by_id(invalid_component_id).is_none());
    }

    #[test]
    fn entity_mut_get_by_id_invalid_component_id() {
        let invalid_component_id = ComponentId::new(usize::MAX);

        let mut world = World::new();
        let mut entity = world.spawn_empty();
        assert!(entity.get_by_id(invalid_component_id).is_none());
        assert!(entity.get_mut_by_id(invalid_component_id).is_none());
    }

    #[test]
    fn entity_mut_insert_by_id() {
        let mut world = World::new();
        let test_component_id = world.init_component::<TestComponent>();

        let mut entity = world.spawn_empty();
        OwningPtr::make(TestComponent(42), |ptr| {
            // SAFETY: `ptr` matches the component id
            unsafe { entity.insert_by_id(test_component_id, ptr) };
        });

        assert_eq!(entity.get::<TestComponent>().unwrap().0, 42);
    }

    #[test]
    fn entity_mut_insert_bundle_by_ids() {
        let mut world = World::new();
        let test_component_id = world.init_component::<TestComponent>();
        let test_component_2_id = world.init_component::<TestComponent2>();

        let mut entity = world.spawn_empty();

        let component_ids = vec![test_component_id, test_component_2_id];
        let test_component_value = TestComponent(42);
        let test_component_2_value = TestComponent2(84);

        OwningPtr::make(test_component_value, |ptr1| {
            OwningPtr::make(test_component_2_value, |ptr2| {
                // SAFETY: `ptr1` and `ptr2` match the component ids
                unsafe { entity.insert_bundle_by_ids(component_ids, vec![ptr1, ptr2]) };
            });
        });

        assert_eq!(entity.get::<TestComponent>().unwrap().0, 42);
        assert_eq!(entity.get::<TestComponent2>().unwrap().0, 84);
    }
}
