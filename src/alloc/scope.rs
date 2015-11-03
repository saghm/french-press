use std::cell::RefCell;
use std::collections::hash_map::HashMap;
use std::collections::hash_set::HashSet;
use std::rc::Rc;
use std::cmp;
use std::mem;

use js_types::js_type::{JsVar, JsType, JsPtrEnum};
use uuid::Uuid;

// Initial Arena size in bytes
const INITIAL_SIZE: usize = 1024;
// Minimum Arena capacity is at least 1 byte
const MIN_CAP: usize = 1;

pub struct Scope {
    parent: Option<Rc<Scope>>,
    children: Vec<Box<Scope>>,
    black_set: HashMap<Uuid, RefCell<JsVar>>,
    grey_set: HashMap<Uuid, RefCell<JsVar>>,
    white_set: HashMap<Uuid, RefCell<JsVar>>,
    get_roots: Box<Fn() -> HashSet<Uuid>>,
}

impl Scope {
    pub fn new<F>(get_roots: F) -> Scope
        where F: Fn() -> HashSet<Uuid> + 'static {
        Scope {
            parent: None,
            children: Vec::new(),
            black_set: HashMap::new(),
            grey_set: HashMap::new(),
            white_set: HashMap::new(),
            get_roots: Box::new(get_roots),
        }
    }

    pub fn as_child<F>(parent: Rc<Scope>, get_roots: F) -> Scope
        where F: Fn() -> HashSet<Uuid> + 'static {
        Scope {
            parent: Some(parent),
            children: Vec::new(),
            black_set: HashMap::new(),
            grey_set: HashMap::new(),
            white_set: HashMap::new(),
            get_roots: Box::new(get_roots),
        }
    }

    pub fn set_parent(&mut self, parent: Rc<Scope>) {
        self.parent = Some(parent);
    }

    pub fn add_child(&mut self, child: Scope) {
        self.children.push(Box::new(child));
    }

    pub fn alloc(&mut self, var: JsVar) -> Uuid {
        let uuid = var.uuid;
        self.white_set.insert(uuid, RefCell::new(var));
        uuid
    }

    pub fn dealloc(&mut self, uuid: &Uuid) -> bool {
        if let Some(_) = self.white_set.remove(uuid) { true } else { false }
    }

    pub fn get_var_copy(&self, uuid: &Uuid) -> Option<JsVar> {
        self.find_id(uuid).map(|var| var.clone().into_inner())
    }

    pub fn update_var(&mut self, var: JsVar) -> bool {
        unimplemented!()
    }

    /// TODO Compute the roots of the current scope-- any variable that is
    /// directly referenced or declared within the scope. This might just be the
    /// key set of the uuid map(?) Not necessarily, I think. What if you do
    /// something like this:
    /// var x = {}
    /// var y = { 1: x }
    /// y = x
    /// y would be a root by the definition above, but is no longer reachable at
    /// the end of the scope because it now aliases x. A better definition would
    /// be "Any variable that is declared or referenced directly, but a direct
    /// reference (variable usage) supercedes a declaration." The above example
    /// demonstrates why this is necessary.
    /// This should come from the interpreter, so I shouldn't actually have to
    /// care about getting the root set myself.

    //pub fn compute_roots(&self) -> HashSet<Uuid> {
    //    self.get_roots();
    //}

    /// Roots always get marked as Black, since they're always reachable from
    /// the current scope. NB that this assumes all root references are actually
    /// valid reference types, i.e. they're not numbers, etc.
    pub fn mark_roots(&mut self, marks: HashSet<Uuid>) {
        for mark in marks.iter() {
            if let Some(var) = self.white_set.remove(mark) {
                let uuid = var.borrow().uuid;
                // Get all child references
                let child_ids = self.get_var_children(&var);
                self.black_set.insert(uuid, var);
                // Mark child references as grey
                self.grey_children(child_ids);
            }
        }
    }

    pub fn mark_phase(&mut self) {
        // Mark any grey object as black, and mark all white objs it refs as grey
        while let Some(&uuid) = self.grey_set.keys().take(1).next() {
            if let Some(var) = self.grey_set.remove(&uuid) {
                let child_ids = self.get_var_children(&var);
                self.black_set.insert(uuid, var);
                for child_id in child_ids {
                    if let Some(var) = self.white_set.remove(&child_id) {
                        self.grey_set.insert(child_id, var);
                    }
                }
            }
        }
    }

    pub fn sweep_phase(&mut self) {
        self.white_set.clear();
        self.white_set.shrink_to_fit();
    }

    fn find_id(&self, uuid: &Uuid) -> Option<&RefCell<JsVar>> {
        self.black_set.get(uuid).or_else(||
            self.grey_set.get(uuid).or_else(||
                self.white_set.get(uuid)))
    }

    fn grey_children(&mut self, child_ids: HashSet<Uuid>) {
        for child_id in child_ids {
            if let Some(var) = self.white_set.remove(&child_id) {
                self.grey_set.insert(child_id, var);
            }
        }
    }

    fn get_var_children(&self, var: &RefCell<JsVar>) -> HashSet<Uuid> {
        if let JsType::JsPtr(ref ptr) = (*var.borrow()).t {
            match ptr {
                &JsPtrEnum::JsObj(ref obj) => obj.get_children(),
                _ => HashSet::new(),
            }
        } else { HashSet::new() }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_set::HashSet;
    use std::rc::Rc;
    use js_types::js_type::{JsVar, JsType};
    use uuid::Uuid;

    fn dummy_get_roots() -> HashSet<Uuid> {
        HashSet::new()
    }

    fn make_num(i: f64) -> JsVar {
        JsVar::new(JsType::JsNum(i))
    }

    #[test]
    fn test_new_scope() {
        let mut test_scope = Scope::new(dummy_get_roots);
        assert!(test_scope.parent.is_none());
        assert!(test_scope.black_set.is_empty());
        assert!(test_scope.grey_set.is_empty());
        assert!(test_scope.white_set.is_empty());
        assert_eq!(test_scope.children.len(), 0);
    }

    #[test]
    fn test_as_child_scope() {
        let parent_scope = Scope::new(dummy_get_roots);
        let mut test_scope = Scope::as_child(Rc::new(parent_scope), dummy_get_roots);

        assert!(test_scope.parent.is_some());
        assert!(test_scope.black_set.is_empty());
        assert!(test_scope.grey_set.is_empty());
        assert!(test_scope.white_set.is_empty());
        assert_eq!(test_scope.children.len(), 0);
    }

    #[test]
    fn test_set_parent() {
        let parent_scope = Scope::new(dummy_get_roots);
        let mut test_scope = Scope::new(dummy_get_roots);
        assert!(test_scope.parent.is_none());
        test_scope.set_parent(Rc::new(parent_scope));
        assert!(test_scope.parent.is_some());
    }

    #[test]
    fn test_add_child() {
        let mut test_scope = Scope::new(dummy_get_roots);
        let child_scope1 = Scope::new(dummy_get_roots);
        let child_scope2 = Scope::new(dummy_get_roots);
        assert_eq!(test_scope.children.len(), 0);
        test_scope.add_child(child_scope1);
        assert_eq!(test_scope.children.len(), 1);
        test_scope.add_child(child_scope2);
        assert_eq!(test_scope.children.len(), 2);
    }

    #[test]
    fn test_alloc() {
        let mut test_scope = Scope::new(dummy_get_roots);
        let test_var = make_num(1.0);
        let test_uuid = test_var.uuid.clone();
        let uuid = test_scope.alloc(test_var);
        assert_eq!(test_uuid, uuid);
        assert!(test_scope.white_set.contains_key(&uuid));
        assert_eq!(test_scope.white_set.len(), 1);
        assert_eq!(test_scope.grey_set.len(), 0);
        assert_eq!(test_scope.black_set.len(), 0);
    }

    #[test]
    fn test_dealloc() {
        let mut test_scope = Scope::new(dummy_get_roots);
        let test_var = make_num(1.0);
        let uuid = test_scope.alloc(test_var);
        let bad_uuid = Uuid::new_v4();
        assert!(test_scope.dealloc(&uuid));
        assert_eq!(test_scope.white_set.len(), 0);
        assert_eq!(test_scope.grey_set.len(), 0);
        assert_eq!(test_scope.black_set.len(), 0);
        assert!(!test_scope.dealloc(&bad_uuid));
    }

    #[test]
    fn test_get_var_copy() {
        let mut test_scope = Scope::new(dummy_get_roots);
        let test_var = make_num(1.0);
        let uuid = test_scope.alloc(test_var);
        let bad_uuid = Uuid::new_v4();
        let var_copy = test_scope.get_var_copy(&uuid);
        assert!(var_copy.is_some());
        let var = var_copy.unwrap();
        assert_eq!(var.uuid, uuid);
        let bad_copy = test_scope.get_var_copy(&bad_uuid);
        assert!(bad_copy.is_none());
    }

    #[test]
    fn test_update_var() {
        let mut test_scope = Scope::new(dummy_get_roots);
        let test_var = make_num(1.0);
        let uuid = test_scope.alloc(test_var);
        let mut update = test_scope.get_var_copy(&uuid).unwrap();
        update = make_num(2.0);
        assert!(test_scope.update_var(update));
        let update = test_scope.get_var_copy(&uuid).unwrap();
        match update {
            JsVar{ t: JsType::JsNum(i), ..} => assert_eq!(i, 2.0),
            _ => ()
        }
        test_scope.dealloc(&uuid);
        assert!(!test_scope.update_var(update));
    }

    #[test]
    fn test_mark_roots() {
        let mut test_scope = Scope::new(dummy_get_roots);
    }
}