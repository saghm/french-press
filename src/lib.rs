#![feature(associated_consts)]
#![feature(drain)]

extern crate uuid;

mod js_types;
mod alloc;

use std::cell::RefCell;
use std::collections::hash_set::HashSet;
use std::rc::Rc;

use uuid::Uuid;

use alloc::AllocBox;
use alloc::scope::Scope;
use js_types::js_type::{JsPtrEnum, JsVar};

pub struct ScopeManager {
    curr_scope: Rc<Scope>,
    alloc_box: Rc<RefCell<AllocBox>>
}

impl ScopeManager {
    pub fn new<F>(alloc_box: Rc<RefCell<AllocBox>>, callback: F) -> ScopeManager
        where F: Fn() -> HashSet<Uuid> + 'static {
        ScopeManager {
            curr_scope: Rc::new(Scope::new(&alloc_box, callback)),
            alloc_box: alloc_box,
        }
    }

    pub fn push_scope<F>(&mut self, callback: F) where F: Fn() -> HashSet<Uuid> + 'static {
        self.curr_scope = Rc::new(Scope::as_child(&self.curr_scope, &self.alloc_box, callback));
    }

    pub fn pop_scope(&mut self) {
        if let Some(parent) = self.curr_scope.parent.clone() {
            // Set curr_scope to old scope's parent
            self.curr_scope = parent;
        } else {
            panic!("Tried to pop to parent scope, but parent did not exist!");
        }
    }

    pub fn alloc(&mut self, var: JsVar, ptr: Option<JsPtrEnum>) -> Uuid {
        Rc::get_mut(&mut self.curr_scope).unwrap().push(var, ptr)
    }

    pub fn load(&self, uuid: &Uuid) -> Result<(JsVar, Option<JsPtrEnum>), String> {
        if let (Some(v), ptr) = self.curr_scope.get_var_copy(uuid) {
            Ok((v, ptr))
        } else { Err(format!("Lookup of uuid {} failed!", uuid)) }
    }

    pub fn store(&mut self, var: JsVar, ptr: Option<JsPtrEnum>) -> bool {
        //Rc::get_mut(&mut self.curr_scope).unwrap().update_ptr(uuid, ptr)
        Rc::get_mut(&mut self.curr_scope).unwrap().update_var(var, ptr)
    }

    fn mark_vars(&mut self) {
        self.alloc_box.borrow_mut().mark_roots((self.curr_scope.get_roots)());
        self.alloc_box.borrow_mut().mark_ptrs();
    }

    fn sweep_ptrs(&mut self) {
        self.alloc_box.borrow_mut().sweep_ptrs();
    }
}

pub fn init_gc<F>(callback: F) -> ScopeManager
    where F: Fn() -> HashSet<Uuid> + 'static {
    let alloc_box = Rc::new(RefCell::new(AllocBox::new()));
    ScopeManager::new(alloc_box, callback)
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_set::HashSet;
    use std::ptr::null_mut;
    use std::rc::Rc;
    use uuid::Uuid;

    fn dummy_callback() -> HashSet<Uuid> {
        HashSet::new()
    }
    // TODO
}
