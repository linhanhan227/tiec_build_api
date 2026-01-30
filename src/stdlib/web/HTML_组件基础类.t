包名 结绳.组件

类 可视化组件
    @code
    _element;

    constructor(element) {
        this._element = element;
        this._element.addEventListener("click", () => {
            this.#被单击();
        });
    }

    getElement() {
        return this._element;
    }
    @end

    @虚拟事件
    方法 创建完毕()
    结束 方法

    属性写 宽度(值: 对象)
        code this.getElement().style.width = #值;
    结束 属性

    属性写 边距(值: 对象)
        code this.getElement().style.padding = (typeof #值 === "number") ? `${#值}px` : #值;
    结束 属性

    属性写 背景颜色(颜色: 对象)
        code this.getElement().style.backgroundColor = #颜色;
    结束 属性

    属性写 CSS样式(样式: CSS样式)
        code this.getElement().className = #样式;
    结束 属性

    定义事件 被单击()
结束 类

类 布局组件 : 可视化组件
    @code
    constructor(element) {
        super(element);
    }
    @end

    方法 添加组件(组件: 可视化组件)
        code this.getElement().append(#组件.getElement());
    结束 方法
结束 类

类 自定义组件
    @code
    let _root;

    constructor() {
        onInit();
    }

    setRootLayout(component) {
        this._root = component;
    }

    getRootLayout() {
        return this._root;
    }

    onInit() {
    }
    @end
结束 类
