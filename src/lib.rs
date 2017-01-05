use std::ptr;
use std::sync::atomic::fence;
use std::sync::atomic::{AtomicPtr, AtomicBool};
use std::sync::atomic::Ordering::{Release, Acquire, Relaxed};
use ::LockState::*;

pub enum LockState {
    Free,
    GuestAcquired,
    RegularAcquired(*mut Node),
}

pub struct Node {
    next: AtomicPtr<Node>,
    waiting: AtomicBool,
}

pub struct MSCg {
    tail: AtomicPtr<Node>,
    guest_tail_node: Node,
}

impl Node {
    pub fn new() -> Node {
        Node {
            next: AtomicPtr::new(ptr::null_mut()),
            waiting: AtomicBool::new(false),
        }
    }
}

impl MSCg {
    pub fn new() -> MSCg {
        MSCg {
            tail: AtomicPtr::new(ptr::null_mut()),
            guest_tail_node: Node::new(),
        }
    }

    fn swap_tail(&mut self, p: *mut Node) -> LockState {
        let pred = self.tail.swap(p, Relaxed);
        if pred.is_null() {
            Free
        } else if pred == &mut self.guest_tail_node {
            GuestAcquired
        } else {
            RegularAcquired(pred)
        }
    }

    pub fn lock(&mut self, p: &mut Node) {
        // Setup local node
        p.next = AtomicPtr::new(ptr::null_mut());
        p.waiting.store(true, Relaxed);

        // tail_node is either your local node or guest_tail_node.
        let mut tail_node = p as *mut Node;

        loop {
            // Place your local node at the tail of the queue
            let state = self.swap_tail(tail_node);

            match state {
                Free => break,
                GuestAcquired => {
                    unsafe { tail_node = &mut *self.tail.swap(&mut self.guest_tail_node, Relaxed); }
                },
                RegularAcquired(prev) => {
                    p.waiting.store(true, Relaxed);
                    unsafe { (*prev).next.store(p, Release); }

                    while p.waiting.load(Relaxed) {
                    }
                    break;
                }
            }
        }

        // Make sure that previous read/write occurs before entering critical section.
        // It might be the case that I am misuing fence()?
        fence(Acquire);
    }

    pub fn unlock(&mut self, p: &mut Node) {
        fence(Release);

        // Get the waiting thread's node if available.
        let mut succ = p.next.load(Relaxed);

        if succ.is_null() {
            // Try to place the null node on tail.
            // If it succeeds, just return.
            // Usually, tail == p because this thread is acquiring the lock and there is no waiting thread (succ.is_null()).
            if self.tail.compare_and_swap(p, ptr::null_mut(), Relaxed) == p {
                return;
            }
            // If the above CAS failed, it means that there is other thread trying to acquire the lock right now.
            // That case, wait until his node is set to p.next.
            while succ.is_null() {
                succ = p.next.load(Relaxed);
            }
        }

        // Release the lock: let the next node acquire the lock.
        unsafe { (*succ).waiting.store(false, Relaxed); }
    }

    pub fn glock(&mut self) {
        while self.tail.compare_and_swap(ptr::null_mut(), &mut self.guest_tail_node, Relaxed).is_null() {
        }
    }

    pub fn gunlock(&mut self) {
        while self.tail.compare_and_swap(&mut self.guest_tail_node, ptr::null_mut(), Relaxed) == &mut self.guest_tail_node {
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MSCg;
    use super::Node;

    #[test]
    fn lock_unlock() {
        let mut l = MSCg::new();
        let mut p = Node::new();
        l.lock(&mut p);
        l.unlock(&mut p);
        l.glock();
        l.gunlock();
    }
}

