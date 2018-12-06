# gpu_linked_list
把你的数据存到GPU内存的普通链表。以侵入式容器方式实现，卫星数据存入GPU内存，原理是使用Vulkan API创建类似于`Box`的`GpuBox`，并在里面存入数据，使用pop方法读出时暂时缓存到CPU。仅供学习研究和娱乐使用。

感谢小老弟们 @AlaricGilbert @ManiaciaChao
