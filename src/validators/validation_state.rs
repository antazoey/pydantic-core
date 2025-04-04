use pyo3::prelude::*;
use pyo3::types::PyString;

use jiter::{PartialMode, StringCacheMode};

use crate::recursion_guard::{ContainsRecursionState, RecursionState};
use crate::tools::new_py_string;

use super::Extra;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub enum Exactness {
    Lax,
    Strict,
    Exact,
}

pub struct ValidationState<'a, 'py> {
    pub recursion_guard: &'a mut RecursionState,
    pub exactness: Option<Exactness>,
    // This is used as a tie-breaking mechanism for union validation.
    // Note: the count of the fields set is not always equivalent to the length of the
    // `model_fields_set` attached to a model. `model_fields_set` includes extra fields
    // when extra='allow', whereas this tally does not.
    pub fields_set_count: Option<usize>,
    // True if `allow_partial=true` and we're validating the last element of a sequence or mapping.
    pub allow_partial: PartialMode,
    // deliberately make Extra readonly
    extra: Extra<'a, 'py>,
}

impl<'a, 'py> ValidationState<'a, 'py> {
    pub fn new(extra: Extra<'a, 'py>, recursion_guard: &'a mut RecursionState, allow_partial: PartialMode) -> Self {
        Self {
            recursion_guard, // Don't care about exactness unless doing union validation
            exactness: None,
            fields_set_count: None,
            allow_partial,
            extra,
        }
    }

    /// Temporarily rebinds the extra field by calling `f` to modify extra.
    ///
    /// When `ValidationStateWithReboundExtra` drops, the extra field is restored to its original value.
    pub fn rebind_extra<'state>(
        &'state mut self,
        f: impl FnOnce(&mut Extra<'a, 'py>),
    ) -> ValidationStateWithReboundExtra<'state, 'a, 'py> {
        let old_extra = self.extra.clone();
        f(&mut self.extra);
        ValidationStateWithReboundExtra { state: self, old_extra }
    }

    pub fn extra(&self) -> &'_ Extra<'a, 'py> {
        &self.extra
    }

    pub fn enumerate_last_partial<I>(&self, iter: impl Iterator<Item = I>) -> impl Iterator<Item = (usize, bool, I)> {
        EnumerateLastPartial::new(iter, self.allow_partial)
    }

    pub fn strict_or(&self, default: bool) -> bool {
        self.extra.strict.unwrap_or(default)
    }

    pub fn validate_by_alias_or(&self, default: Option<bool>) -> bool {
        self.extra.by_alias.or(default).unwrap_or(true)
    }

    pub fn validate_by_name_or(&self, default: Option<bool>) -> bool {
        self.extra.by_name.or(default).unwrap_or(false)
    }

    /// Sets the exactness to the lower of the current exactness
    /// and the given exactness.
    ///
    /// This is designed to be used in union validation, where the
    /// idea is that the "most exact" validation wins.
    pub fn floor_exactness(&mut self, exactness: Exactness) {
        match self.exactness {
            None | Some(Exactness::Lax) => {}
            Some(Exactness::Strict) => {
                if exactness == Exactness::Lax {
                    self.exactness = Some(Exactness::Lax);
                }
            }
            Some(Exactness::Exact) => self.exactness = Some(exactness),
        }
    }

    pub fn add_fields_set(&mut self, fields_set_count: usize) {
        *self.fields_set_count.get_or_insert(0) += fields_set_count;
    }

    pub fn cache_str(&self) -> StringCacheMode {
        self.extra.cache_str
    }

    pub fn maybe_cached_str(&self, py: Python<'py>, s: &str) -> Bound<'py, PyString> {
        new_py_string(py, s, self.extra.cache_str)
    }
}

impl ContainsRecursionState for ValidationState<'_, '_> {
    fn access_recursion_state<R>(&mut self, f: impl FnOnce(&mut RecursionState) -> R) -> R {
        f(self.recursion_guard)
    }
}

pub struct ValidationStateWithReboundExtra<'state, 'a, 'py> {
    state: &'state mut ValidationState<'a, 'py>,
    old_extra: Extra<'a, 'py>,
}

impl<'a, 'py> std::ops::Deref for ValidationStateWithReboundExtra<'_, 'a, 'py> {
    type Target = ValidationState<'a, 'py>;

    fn deref(&self) -> &Self::Target {
        self.state
    }
}

impl std::ops::DerefMut for ValidationStateWithReboundExtra<'_, '_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.state
    }
}

impl Drop for ValidationStateWithReboundExtra<'_, '_, '_> {
    fn drop(&mut self) {
        std::mem::swap(&mut self.state.extra, &mut self.old_extra);
    }
}

/// Similar to `iter.enumerate()` but also returns a bool indicating if we're at the last element.
pub struct EnumerateLastPartial<I: Iterator> {
    iter: I,
    index: usize,
    next_item: Option<I::Item>,
    allow_partial: PartialMode,
}
impl<I: Iterator> EnumerateLastPartial<I> {
    pub fn new(mut iter: I, allow_partial: PartialMode) -> Self {
        let next_item = iter.next();
        Self {
            iter,
            index: 0,
            next_item,
            allow_partial,
        }
    }
}

impl<I: Iterator> Iterator for EnumerateLastPartial<I> {
    type Item = (usize, bool, I::Item);

    fn next(&mut self) -> Option<Self::Item> {
        let a = std::mem::replace(&mut self.next_item, self.iter.next())?;
        let i = self.index;
        self.index += 1;
        Some((i, self.allow_partial.is_active() && self.next_item.is_none(), a))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}
