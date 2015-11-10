#![feature(associated_consts)]
#![feature(drain)]

extern crate uuid;
extern crate typed_arena;
mod js_types;
mod alloc;

use std::collections::hash_set::HashSet;
use std::ptr::null_mut;
use std::rc::Rc;
use uuid::Uuid;

use alloc::scope::Scope;
use js_types::js_type::JsVar;

pub struct ScopeManager {
    root_scope: Rc<Scope>,
    curr_scope: *mut Rc<Scope>,
}

impl ScopeManager {
    pub fn new<F>(callback: F) -> ScopeManager where F: Fn() -> HashSet<Uuid> + 'static {
        let scope = Rc::new(Scope::new(callback));
        let mut mgr =
            ScopeManager {
                root_scope: scope,
                curr_scope: null_mut(),
            };
        mgr.curr_scope = &mut (mgr.root_scope) as *mut Rc<Scope>;
        mgr
    }

    pub fn add_scope<F>(&mut self, callback: F) where F: Fn() -> HashSet<Uuid> + 'static {
        unsafe {
            let weak_clone = Rc::downgrade(&*self.curr_scope.clone());
            self.curr_scope =
                Rc::get_mut(&mut *self.curr_scope)
                    .unwrap()
                    .add_child(Scope::as_child(weak_clone, callback)) as *mut Rc<Scope>;
        }
    }

    pub fn alloc(&mut self, var: JsVar) -> Uuid {
        unsafe {
            Rc::get_mut(&mut *self.curr_scope).unwrap().alloc(var)
        }
    }
}

pub fn init<F>(callback: F) -> ScopeManager
    where F: Fn() -> HashSet<Uuid> + 'static {
    ScopeManager::new(callback)
}

pub fn load(scope: &Scope, uuid: Uuid) -> Option<JsVar> {
    scope.get_var_copy(&uuid)
}

pub fn store(scope: &mut Scope, var: JsVar) -> bool {
    scope.update_var(var)
}
