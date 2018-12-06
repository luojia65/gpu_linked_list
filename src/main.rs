#![feature(box_into_raw_non_null)]
use vulkano::memory::DeviceMemory;
use vulkano::memory::MappedDeviceMemory;
use vulkano::device::Device;
use vulkano::device::DeviceExtensions;
use vulkano::device::Features;
use vulkano::instance::Instance;
use vulkano::instance::InstanceExtensions;
use vulkano::instance::PhysicalDevice;
use std::sync::Arc;
use core::mem::size_of;
use core::ptr;
use core::ptr::NonNull;
use core::marker::PhantomData;
use core::fmt;
use core::iter::FusedIterator;

pub struct LinkedList<T> {
    head: Option<NonNull<Node<T>>>,
    tail: Option<NonNull<Node<T>>>,
    len: usize,
    device: Arc<Device>,
    _marker: PhantomData<Box<Node<T>>>  
}

struct GpuBox<T> {
    inner: MappedDeviceMemory,
    _marker: PhantomData<T>,
}

impl<T> GpuBox<T> {
    fn new(data: T, device: Arc<Device>) -> Self {
        let mem_ty = device.physical_device().memory_types()
                            .filter(|t| t.is_host_visible())
                            .next().unwrap();  
        let memory = DeviceMemory::alloc_and_map(device.clone(), mem_ty, size_of::<T>()).unwrap();

        unsafe {
            let mut content = memory.read_write::<T>(0..size_of::<T>());
            *content = data;
        }
        
        GpuBox {
            inner: memory,
            _marker: PhantomData,
        }
    }

    fn into_inner(self) -> T {
        unsafe {
            ptr::read(self.as_ref())
        }
    }

    fn as_ref(&self) -> &T {
        unsafe {
            let b = Box::new(self.inner.read_write::<T>(0..size_of::<T>()));
            Box::leak(b)
        }
    }
}

struct Node<T> {
    prev: Option<NonNull<Node<T>>>,
    next: Option<NonNull<Node<T>>>,
    data: GpuBox<T>,
}

impl<T> LinkedList<T> {
    pub fn new() -> Self {
        let instance = Instance::new(None, &InstanceExtensions::none(), None)
            .expect("failed to create instance");
        let physical = PhysicalDevice::enumerate(&instance).next().expect("no device available");
        let queue_family = physical.queue_families()
            .find(|&q| q.supports_graphics())
            .expect("couldn't find a graphical queue family");

        let (device, mut _queues) = {
            Device::new(physical, &Features::none(), &DeviceExtensions::none(),
                        [(queue_family, 0.5)].iter().cloned()).expect("failed to create device")
        };
        Self::with_device(device)
    }

    fn with_device(device: Arc<Device>) -> Self {
        LinkedList {
            head: None,
            tail: None,
            len: 0,
            device,
            _marker: PhantomData
        }
    }

    pub fn len(&self) -> usize {
        self.len
    } 

    pub fn iter(&self) -> Iter<T> {
        Iter {
            head: self.head,
            tail: self.tail,
            len: self.len,
            _marker: PhantomData,
        }
    }
}

macro_rules! push_pop_impl {
    ($inner_type: ty, $push_fn: ident, $pop_fn: ident, 
    $new_node_from: ident, $old_node_from: ident, $new_node_dir: ident, $old_node_dir: ident) => {
        
    pub fn $push_fn(&mut self, data: $inner_type) {
        let mut new_node = Box::new(Node {
            prev: None,
            next: None,
            data: GpuBox::new(data, self.device.clone()),
        });
        new_node.$old_node_dir = self.$old_node_from;
        let new_node = Some(Box::into_raw_non_null(new_node));
        match self.$old_node_from {
            Some(mut node) => unsafe { node.as_mut() } .$new_node_dir = new_node,
            None => self.$new_node_from = new_node
        }
        self.$old_node_from = new_node;
        self.len += 1;
    }

    pub fn $pop_fn(&mut self) -> Option<$inner_type> {
        if let Some(old_node) = self.$old_node_from {
            let old_node = unsafe { Box::from_raw(old_node.as_ptr()) };
            self.$old_node_from = old_node.$old_node_dir;
            match self.$old_node_from {
                Some(mut old_ptr) => unsafe { old_ptr.as_mut() }.$new_node_dir = None,
                None => self.$new_node_from = None
            }
            self.len -= 1;
            Some(old_node.data.into_inner())
        } else {
            None
        }
    }

    };
}

impl<T> LinkedList<T> {
    push_pop_impl!(T, push_back,  pop_back,  head, tail, next, prev);
    push_pop_impl!(T, push_front, pop_front, tail, head, prev, next);
}

impl<T: fmt::Debug> fmt::Debug for LinkedList<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T> Default for LinkedList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for LinkedList<T> {
    fn drop(&mut self) {
        while let Some(_) = self.pop_back() {}
    }
}


pub struct Iter<'a, T: 'a> {
    head: Option<NonNull<Node<T>>>,
    tail: Option<NonNull<Node<T>>>,
    len: usize,
    _marker: PhantomData<&'a Node<T>>,
}

impl<'a, T: 'a + fmt::Debug> fmt::Debug for Iter<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple("Iter")
         .field(&self.len)
         .finish()
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        if self.len == 0 {
            None
        } else {
            self.head.map(|node| unsafe {
                let node = &*node.as_ptr();
                self.len -= 1;
                self.head = node.next;
                node.data.as_ref()
            })
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl<'a, T> DoubleEndedIterator for Iter<'a, T> {
    #[inline]
    fn next_back(&mut self) -> Option<&'a T> {
        if self.len == 0 {
            None
        } else {
            self.tail.map(|node| unsafe {
                let node = &*node.as_ptr(); // unbounded lifetime
                self.len -= 1;
                self.tail = node.prev;
                node.data.as_ref()
            })
        }
    }
}

impl<'a, T> ExactSizeIterator for Iter<'a, T> {}

impl<'a, T> FusedIterator for Iter<'a, T> {}

#[cfg(test)]
mod linked_list_tests {

    use super::*;
    #[test]
    fn test_push_pop() {
        let mut list = LinkedList::new();
        list.push_front(3);
        list.push_front(2);
        list.push_front(1);
        assert_eq!(Some(3), list.pop_back());
        assert_eq!(Some(2), list.pop_back());
        assert_eq!(Some(1), list.pop_back());
        assert_eq!(None, list.pop_back());
        let mut list = LinkedList::new();
        list.push_back(1);
        list.push_back(2);
        list.push_back(3);
        assert_eq!(Some(1), list.pop_front());
        assert_eq!(Some(2), list.pop_front());
        assert_eq!(Some(3), list.pop_front());
        assert_eq!(None, list.pop_front());
    }

    #[test]
    fn test_drop() {
        struct Data(u8);
        impl Drop for Data {
            fn drop(&mut self) {
                println!("dropped {}", self.0)
            }
        }
        let mut list = LinkedList::new();
        list.push_back(Data(1));
        list.push_back(Data(2));
        list.push_back(Data(3));
        // Now list is out of scope
    }
}
