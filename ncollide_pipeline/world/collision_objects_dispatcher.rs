use std::slice::Iter;
use utils::data::hash_map::{HashMap, Entry};
use utils::data::pair::{Pair, PairTWHash};
use utils::data::uid_remap::{UidRemap, FastKey};
use queries::geometry::Contact;
use narrow_phase::{CollisionDispatcher, CollisionAlgorithm, ContactSignal, ContactSignalHandler,
                   CollisionDetector};
use world::CollisionObject;
use math::Point;

// FIXME: move this to the `narrow_phase` module.
/// Collision detector dispatcher for collision objects.
pub struct CollisionObjectsDispatcher<P, M, T> {
    signal:           ContactSignal<T>,
    shape_dispatcher: Box<CollisionDispatcher<P, M> + 'static>,
    pairs:            HashMap<Pair, CollisionAlgorithm<P, M>, PairTWHash>
}

impl<P: Point, M: 'static, T> CollisionObjectsDispatcher<P, M, T> {
    /// Creates a new `CollisionObjectsDispatcher`.
    pub fn new(shape_dispatcher: Box<CollisionDispatcher<P, M> + 'static>)
        -> CollisionObjectsDispatcher<P, M, T> {
        CollisionObjectsDispatcher {
            signal:           ContactSignal::new(),
            pairs:            HashMap::new(PairTWHash::new()),
            shape_dispatcher: shape_dispatcher
        }
    }

    /// Updates the contact pairs.
    pub fn update(&mut self, objects: &UidRemap<CollisionObject<P, M, T>>, timestamp: usize) {
        for e in self.pairs.elements_mut().iter_mut() {
            let co1 = &objects[e.key.first];
            let co2 = &objects[e.key.second];

            if co1.timestamp == timestamp || co2.timestamp == timestamp {
                let had_colls = e.value.num_colls() != 0;

                e.value.update(&*self.shape_dispatcher,
                               &co1.position, &**co1.shape,
                               &co2.position, &**co2.shape);

                if e.value.num_colls() == 0 {
                    if had_colls {
                        self.signal.trigger_contact_signal(&co1.data, &co2.data, false);
                    }
                }
                else {
                    if !had_colls {
                        self.signal.trigger_contact_signal(&co1.data, &co2.data, true)
                    }
                }
            }
        }
    }

    /// Iterates through all the contact pairs.
    #[inline]
    pub fn contact_pairs<'a>(&'a self, objects: &'a UidRemap<CollisionObject<P, M, T>>)
                             -> ContactPairs<'a, P, M, T> {
        ContactPairs {
            objects: objects,
            pairs:   self.pairs.elements().iter()
        }
    }

    /// Iterates through all the contacts detected since the last update.
    #[inline]
    pub fn contacts<'a>(&'a self, objects: &'a UidRemap<CollisionObject<P, M, T>>) -> Contacts<'a, P, M, T> {
        Contacts {
            objects:      objects,
            co1:          None,
            co2:          None,
            pairs:        self.pairs.elements().iter(),
            collector:    Vec::new(), // FIXME: avoid allocations.
            curr_contact: 0
        }
    }

    /// Registers a handler for contact start/stop events.
    pub fn register_contact_signal_handler(&mut self,
                                           name: &str,
                                           handler: Box<ContactSignalHandler<T> + 'static>) {
        self.signal.register_contact_signal_handler(name, handler)
    }

    /// Unregisters a handler for contact start/stop events.
    pub fn unregister_contact_signal_handler(&mut self, name: &str) {
        self.signal.unregister_contact_signal_handler(name)
    }

    /// Creates/removes the persistant collision detector associated to a given pair of objects.
    pub fn handle_proximity(&mut self,
                            objects: &UidRemap<CollisionObject<P, M, T>>,
                            fk1: &FastKey,
                            fk2: &FastKey,
                            started: bool) {
        let key = Pair::new(*fk1, *fk2);

        if started {
            let cd;

            {
                let co1 = &objects[*fk1];
                let co2 = &objects[*fk2];
                cd = self.shape_dispatcher.get_collision_algorithm(&co1.shape.repr(), &co2.shape.repr());
            }

            if let Some(cd) = cd {
                let _ = self.pairs.insert(key, cd);
            }
        }
        else {
            // Proximity stopped.
            match self.pairs.get_and_remove(&key) {
                Some(detector) => {
                    // Trigger the collision lost signal if there was a contact.
                    if detector.value.num_colls() != 0 {
                        let co1 = &objects[*fk1];
                        let co2 = &objects[*fk2];

                        self.signal.trigger_contact_signal(&co1.data, &co2.data, false);
                    }
                },
                None => { }
            }
        }
    }

    /// Tests if two objects can be tested for mutual collision.
    pub fn is_proximity_allowed(objects: &UidRemap<CollisionObject<P, M, T>>,
                                fk1: &FastKey,
                                fk2: &FastKey) -> bool {
        let co1 = &objects[*fk1];
        let co2 = &objects[*fk2];

        let can_move_ok = true; // XXX: ba.can_move() || bb.can_move();
        let groups_ok = co1.collision_groups.can_collide_with_groups(&co2.collision_groups);

        if *fk1 == *fk2 {
            can_move_ok && co1.collision_groups.can_collide_with_self()
        }
        else {
            can_move_ok && groups_ok
        }
    }
}

/// Iterator through contact pairs.
pub struct ContactPairs<'a, P: 'a, M: 'a, T: 'a> {
    objects: &'a UidRemap<CollisionObject<P, M, T>>,
    pairs:   Iter<'a, Entry<Pair, Box<CollisionDetector<P, M> + 'static>>>
}

impl<'a, P, M, T> Iterator for ContactPairs<'a, P, M, T> {
    type Item = (&'a CollisionObject<P, M, T>,
                 &'a CollisionObject<P, M, T>,
                 &'a CollisionAlgorithm<P, M>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self.pairs.next() {
            Some(p) => {
                let co1 = &self.objects[p.key.first];
                let co2 = &self.objects[p.key.second];

                Some((&co1, &co2, &p.value))
            }
            None => None
        }
    }
}

    /// An iterator through contacts.
    pub struct Contacts<'a, P: 'a + Point, M: 'a, T: 'a> {
        objects:      &'a UidRemap<CollisionObject<P, M, T>>,
        // FIXME: do we want to pay the cost of Options here? We already know those references will
        // never be null anyway so we could avoid the Option and init them with
        // `mem::uninitialized()` instead.
        co1:          Option<&'a CollisionObject<P, M, T>>,
        co2:          Option<&'a CollisionObject<P, M, T>>,
        pairs:        Iter<'a, Entry<Pair, Box<CollisionDetector<P, M>>>>,
        collector:    Vec<Contact<P>>,
        curr_contact: usize
    }

    impl<'a, P: Point, M, T> Iterator for Contacts<'a, P, M, T> {
        type Item = (&'a CollisionObject<P, M, T>, &'a CollisionObject<P, M, T>, Contact<P>);

        #[inline]
        fn next(&mut self) -> Option<Self::Item> {
            // FIXME: is there a more efficient way to do this (i-e. avoid using an index)?
            if self.curr_contact < self.collector.len() {
                self.curr_contact = self.curr_contact + 1;

                // FIXME: would be nice to avoid the `clone` and return a reference
                // instead (but what would be its lifetime?).
                Some((self.co1.unwrap(), self.co2.unwrap(), self.collector[self.curr_contact - 1].clone()))
            }
            else {
                self.collector.clear();

                while let Some(p) = self.pairs.next() {
                    p.value.colls(&mut self.collector);

                    if !self.collector.is_empty() {
                        self.co1 = Some(&self.objects[p.key.first]);
                        self.co2 = Some(&self.objects[p.key.second]);
                        self.curr_contact = 1; // Start at 1 instead of 0 because we will return the first one here.

                        // FIXME: would be nice to avoid the `clone` and return a reference
                        // instead (but what would be its lifetime?).
                        return Some((self.co1.unwrap(), self.co2.unwrap(), self.collector[0].clone()))
                    }
                }

                None
            }
        }
    }

