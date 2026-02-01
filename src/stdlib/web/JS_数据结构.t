包名 结绳.基本

类 遍历器<元素类型>
    方法 还有下一个(): 逻辑型
        返回 假
    结束 方法

    方法 下一个(): 元素类型
        返回 空
    结束 方法
结束 类

@禁止创建对象
类 键值表条目<键类型, 值类型>
    属性读 键(): 键类型
        返回 空
    结束 属性

    属性读 值(): 值类型
        返回 空
    结束 属性
结束 类

类 集合遍历器<元素类型> : 遍历器<元素类型>
    @code
    _array;
    _index = 0;
    constructor(array) {
        this._array = array;
    }
    @end

    方法 还有下一个(): 逻辑型
        code return _index < _array.length;
    结束 方法

    方法 下一个(): 元素类型
        code return _array[index++];
    结束 方法
结束 类

@指代类("Array")
类 集合<元素类型 = 对象>
    @静态
    @运算符重载
    方法 =(初始值: 元素类型[]): 集合<元素类型>
        code return Array.from(#初始值);
    结束 方法

    方法 取遍历器(): 集合遍历器<元素类型>
        code return new #cls<集合遍历器<元素类型>>(#this);
    结束 方法
结束 类