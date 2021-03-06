#![feature(associated_consts)]
#![feature(box_patterns)]
#![feature(box_syntax)]
#![feature(question_mark)]
//#![feature(plugin)]

//#![plugin(clippy)]

extern crate jsrs_common;

#[macro_use] extern crate matches;

mod scope;

use std::cell::RefCell;
use std::collections::hash_map::HashMap;
use std::rc::Rc;

use jsrs_common::alloc_box::AllocBox;
use jsrs_common::ast::Exp;
use jsrs_common::backend::Backend;
use jsrs_common::types::js_var::{JsPtrEnum, JsVar};
use jsrs_common::types::binding::{Binding, UniqueBinding};

use jsrs_common::gc_error::{GcError, Result};
use scope::{LookupError, Scope, ScopeTag, StoreError};

pub struct ScopeManager {
    scopes: Vec<Scope>,
    closures: HashMap<UniqueBinding, Scope>,
    pub alloc_box: Rc<RefCell<AllocBox>>,
}

impl ScopeManager {
    fn new(alloc_box: Rc<RefCell<AllocBox>>) -> ScopeManager {
        ScopeManager {
            scopes: vec![Scope::new(ScopeTag::Call, &alloc_box)],
            closures: HashMap::new(),
            alloc_box: alloc_box,
        }
    }

    #[allow(dead_code)]
    #[inline]
    fn curr_scope(&self) -> &Scope {
        self.scopes.last().expect("Tried to access current scope, but none existed")
    }

    #[inline]
    fn curr_scope_mut(&mut self) -> &mut Scope {
        self.scopes.last_mut().expect("Tried to access current scope, but none existed")
    }

    #[inline]
    fn global_scope(&self) -> &Scope {
        self.scopes.get(0).expect("Tried to access global scope, but none existed")
    }

    #[inline]
    fn global_scope_mut(&mut self) -> &mut Scope {
        self.scopes.get_mut(0).expect("Tried to access global scope, but none existed")
    }

    pub fn push_closure_scope(&mut self, closure: &UniqueBinding) -> Result<()> {
        let closure_scope = self.closures.remove(closure).ok_or(GcError::Scope)?;
        self.scopes.push(closure_scope);
        Ok(())
    }

    pub fn push_scope(&mut self, exp: &Exp) {
        let tag = match *exp {
            Exp::Call(..) => ScopeTag::Call,
            _ => ScopeTag::Block,
        };
        self.scopes.push(Scope::new(tag, &self.alloc_box));
    }

    pub fn pop_scope(&mut self, returning_closure: Option<UniqueBinding>, gc_yield: bool) -> Result<()> {
        if let Some(mut scope) = self.scopes.pop() {
            // Clean up the dying scope's stack and take ownership of its heap-allocated data for
            // later collection
            if self.scopes.is_empty() {
                // The global scope was popped and the program is ending.
                scope.trigger_gc();
                return Err(GcError::Scope);
            }
            if let Some(unique) = returning_closure {
                let mut closure_scope = Scope::new(ScopeTag::Closure(unique.clone()), &self.alloc_box);
                scope.transfer_stack(&mut closure_scope, true)?;
                self.closures.insert(unique, closure_scope);
            } else {
                if !matches!(scope.tag, ScopeTag::Closure(_)) {
                    scope.transfer_stack(self.curr_scope_mut(), false)?
                }
            }
            // Potentially trigger the garbage collector
            if gc_yield {
                self.curr_scope_mut().trigger_gc();
            }
            if let ScopeTag::Closure(unique) = scope.tag.clone() {
                self.closures.insert(unique.clone(), scope);
            }
            Ok(())
        } else {
            Err(GcError::Scope)
        }
    }

    pub fn rename_closure(&mut self, old: &UniqueBinding, new: &UniqueBinding) -> bool {
        if self.closures.contains_key(old) {
            let mut scope = self.closures.remove(old).unwrap();
            scope.tag = ScopeTag::Closure(new.clone());
            self.closures.insert(new.clone(), scope);
            true
        } else {
            false
        }
    }
}

impl Backend for ScopeManager {
    fn alloc(&mut self, var: JsVar, ptr: Option<JsPtrEnum>) -> Result<Binding> {
        let binding = var.binding.clone();
        let is_allocated = self.alloc_box.borrow().is_allocated(&var.unique);

        // If the ptr is already allocated in the heap, just push it onto the stack
        if is_allocated && ptr.is_some() {
            self.curr_scope_mut().bind_var(var);
        } else {
            self.curr_scope_mut().push_var(var, ptr)?;
        }
        Ok(binding)
    }

    /// Try to load the variable behind a binding
    fn load(&mut self, bnd: &Binding) -> Result<(JsVar, Option<JsPtrEnum>)> {
        let lookup = || {
            for scope in self.scopes.iter().rev() {
                match scope.get_var_copy(bnd) {
                    Ok(v) => { return Ok(v); },
                    Err(LookupError::FnBoundary) => {
                        return Err(GcError::Load(bnd.clone()));
                    },
                    Err(LookupError::CheckParent) => {},
                    Err(LookupError::Unreachable) => unreachable!(),
                }
            }
            Err(GcError::Load(bnd.clone()))
        };
        match lookup() {
            Ok(v) => Ok(v),
            Err(GcError::Load(bnd)) =>
                self.global_scope().get_var_copy(&bnd)
                .map_err(|_| GcError::Load(bnd.clone())),
            _ => unreachable!(),
        }
    }

    fn store(&mut self, var: JsVar, ptr: Option<JsPtrEnum>) -> Result<()> {
        let (mut var, mut ptr) = (var, ptr);
        let lookup = {
            let mut res = Err(GcError::Store(var.clone(), ptr.clone()));
            for ref mut scope in self.scopes.iter_mut().rev() {
                match scope.update_var(var, ptr) {
                    Ok(()) => {
                        res = Ok(());
                        break;
                    }
                    Err(StoreError::CheckParent(v, p)) => { var = v; ptr = p; },
                    Err(StoreError::FnBoundary(v, p)) => {
                        res = Err(GcError::Store(v, p));
                        break;
                    },
                    Err(StoreError::PtrTypeMismatch) |
                    Err(StoreError::BadStore) => {
                        res = Err(GcError::PtrAlloc);
                        break;
                    },
                }
            }
            res
        };
        match lookup {
            Ok(()) => Ok(()),
            Err(GcError::Store(var, ptr)) =>
                self.global_scope_mut().update_var(var.clone(), ptr.clone())
                    .map_err(|_| GcError::Store(var, ptr)),
            Err(_) => lookup,
        }
        /*let res = self.curr_scope_mut().update_var(var, ptr);
        if let Err(GcError::Store(var, ptr)) = res {
            self.global_scope_mut().update_var(var, ptr)
        } else {
            res
        }*/
    }

    fn get_alloc_box(&self) -> Rc<RefCell<AllocBox>> {
        self.alloc_box.clone()
    }
}

pub fn init_gc() -> ScopeManager {
    let alloc_box = Rc::new(RefCell::new(AllocBox::new()));
    ScopeManager::new(alloc_box)
}


#[cfg(test)]
mod tests {
    use super::*;

    use jsrs_common::ast::Exp;
    use jsrs_common::backend::Backend;
    use jsrs_common::gc_error::GcError;
    use jsrs_common::test_utils;
    use jsrs_common::types::js_var::{JsKey, JsPtrEnum, JsType, JsVar};
    use jsrs_common::types::binding::Binding;

    #[test]
    fn test_push_closure_scope() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        mgr.push_scope(&Exp::Undefined);
        let (fn_var, fn_ptr) = test_utils::make_fn(&None, &Vec::new());
        let unique = fn_var.unique.clone();
        mgr.alloc(fn_var, Some(fn_ptr)).unwrap();
        mgr.pop_scope(Some(unique.clone()), false).unwrap();
        assert_eq!(mgr.closures.len(), 1);
        mgr.push_closure_scope(&unique).unwrap();
        assert_eq!(mgr.closures.len(), 0);
        mgr.pop_scope(None, false).unwrap();
        assert_eq!(mgr.closures.len(), 1);
    }

    #[test]
    fn test_pop_scope() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        mgr.push_scope(&Exp::Undefined);
        assert_eq!(mgr.scopes.len(), 2);
        mgr.pop_scope(None, false).unwrap();
        assert_eq!(mgr.scopes.len(), 1);
    }

    #[test]
    fn test_pop_scope_fail() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        let res = mgr.pop_scope(None, false);
        assert!(res.is_err());
        assert!(matches!(res, Err(GcError::Scope)));
    }

    #[test]
    fn test_alloc() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        mgr.alloc(test_utils::make_num(1.), None).unwrap();
        mgr.push_scope(&Exp::Undefined);
        mgr.alloc(test_utils::make_num(2.), None).unwrap();
        assert!(mgr.alloc_box.borrow().is_empty());
    }

    #[test]
    fn test_load() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        let x = test_utils::make_num(1.);
        let x_bnd = mgr.alloc(x, None).unwrap();
        let load = mgr.load(&x_bnd);
        assert!(load.is_ok());
        let load = load.unwrap();
        match load.0.t {
            JsType::JsNum(n) => assert!(f64::abs(n - 1.) < 0.0001),
            _ => unreachable!(),
        }
        assert!(load.1.is_none());
    }

    #[test]
    fn test_load_fail() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        let bnd = Binding::new("".to_owned());
        let res = mgr.load(&bnd);
        assert!(res.is_err());
        assert!(matches!(res, Err(GcError::Load(_))));
        if let Err(GcError::Load(res_bnd)) = res {
            assert_eq!(bnd, res_bnd);
        }
    }

    #[test]
    fn test_store() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box,);
        mgr.push_scope(&Exp::Undefined);

        let x = test_utils::make_num(1.);
        let x_bnd = mgr.alloc(x, None).unwrap();

        let (mut var, _) = mgr.load(&x_bnd).unwrap();
        var.t = JsType::JsNum(2.);

        assert!(mgr.store(var, None).is_ok());
    }

    #[test]
    fn test_store_fail() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);
        let x = test_utils::make_num(1.);
        assert!(mgr.store(x, None).is_err());
    }

    #[test]
    fn test_store_to_parent_scope() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);

        // Avoids having just the global scope available
        mgr.push_scope(&Exp::Call(box Exp::Undefined, vec![]));
        let x = test_utils::make_num(1.);
        let x_bnd = mgr.alloc(x, None).unwrap();
        let copy = mgr.load(&x_bnd);
        let (mut x, _) = copy.unwrap();

        mgr.push_scope(&Exp::Undefined);
        match x.t {
            JsType::JsNum(_) => x.t = JsType::JsNum(1.),
            _ => unreachable!(),
        };
        assert!(mgr.store(x, None).is_ok())
    }

    #[test]
    fn test_store_to_parent_scope_across_fn_boundary() {
        let alloc_box = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(alloc_box);

        // Avoids having just the global scope available
        mgr.push_scope(&Exp::Call(box Exp::Undefined, vec![]));
        let x = test_utils::make_num(1.);
        let x_bnd = mgr.alloc(x, None).unwrap();
        let copy = mgr.load(&x_bnd);
        let (mut x, _) = copy.unwrap();

        mgr.push_scope(&Exp::Call(box Exp::Undefined, vec![]));
        match x.t {
            JsType::JsNum(_) => x.t = JsType::JsNum(1.),
            _ => unreachable!(),
        };
        assert!(mgr.store(x, None).is_err())
    }

    #[test]
    fn test_load_from_parent_scope_across_fn_boundary() {
        let heap = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(heap);

        // Avoids having just the global scope available
        mgr.push_scope(&Exp::Call(box Exp::Undefined, vec![]));
        let (x, x_ptr) = test_utils::make_str("x");
        let x_bnd = mgr.alloc(x, Some(x_ptr)).unwrap();

        mgr.push_scope(&Exp::Call(box Exp::Undefined, vec![]));
        let copy = mgr.load(&x_bnd);

        assert!(copy.is_err());
        assert!(matches!(copy, Err(GcError::Load(_))));
    }

    #[test]
    fn test_load_from_parent_scope_no_fn_call() {
        let heap = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(heap);

        // Avoids having just the global scope available
        mgr.push_scope(&Exp::Call(box Exp::Undefined, vec![]));
        let (x, x_ptr) = test_utils::make_str("x");
        let x_bnd = mgr.alloc(x, Some(x_ptr)).unwrap();

        mgr.push_scope(&Exp::Undefined);
        let copy = mgr.load(&x_bnd);

        assert!(copy.is_ok());
        let (var_copy, ptr_copy) = copy.unwrap();
        assert!(matches!(var_copy, JsVar { t: JsType::JsPtr(_), .. }));
        assert!(ptr_copy.is_some());
    }

    #[test]
    fn test_transfer_stack_with_yield() {
        let heap = test_utils::make_alloc_box();
        let mut mgr = ScopeManager::new(heap);
        // Make some scopes
        mgr.push_scope(&Exp::Undefined);
        {
            // Push a child scope
            mgr.push_scope(&Exp::Undefined);
            // Allocate some non-root variables (numbers)
            mgr.alloc(test_utils::make_num(0.), None).unwrap();
            mgr.alloc(test_utils::make_num(1.), None).unwrap();
            mgr.alloc(test_utils::make_num(2.), None).unwrap();

            // Make a string to put into an object
            // (so it's heap-allocated and we can lose its ref from the object)
            let (var, ptr) = test_utils::make_str("test");

            // Create an obj of { true: 1.0, false: heap("test") }
            let kvs = vec![(JsKey::JsSym("true".to_string()),
                            test_utils::make_num(1.), None),
                           (JsKey::JsSym("false".to_string()),
                            var, Some(ptr))];
            let (var, ptr) = test_utils::make_obj(kvs, mgr.alloc_box.clone());

            // Push the obj into the current scope
            let bnd = mgr.alloc(var, Some(ptr)).unwrap();
            // The heap should now have 2 things in it: an object and a string
            assert_eq!(mgr.alloc_box.borrow().len(), 2);

            // Replace the string in the object with something else so it's no longer live
            let copy = mgr.load(&bnd);
            let (var_cp, mut ptr_cp) = copy.unwrap();
            let key = JsKey::JsSym("false".to_string());
            match *&mut ptr_cp {
                Some(JsPtrEnum::JsObj(ref mut obj)) => {
                    obj.add_key(&var_cp.unique, key, test_utils::make_num(-1.), None, &mut *(mgr.alloc_box.borrow_mut()));
                },
                _ => unreachable!()
            }
            mgr.store(var_cp, ptr_cp).unwrap();
            // The heap should still have 2 things in it: an object and a string
            assert_eq!(mgr.alloc_box.borrow().len(), 2);

            // Kill the current scope & give its refs to the parent,
            // allowing the GC to kick in beforehand.
            mgr.pop_scope(None, true).unwrap();
        }
        // The object we created above should still exist
        assert_eq!(mgr.curr_scope().len(), 1);
        // But the string it had allocated shouldn't, since we leaked it into the void
        assert_eq!(mgr.alloc_box.borrow().len(), 1);
    }
}
