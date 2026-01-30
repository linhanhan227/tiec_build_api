包名 结绳.网页

类 文本框 : 可视化组件
    @code
    constructor() {
        super(document.createElement("div"));
    }
    @end

    属性写 内容(内容: 文本)
        code this.getElement().innerText = #内容;
    结束 属性

    属性写 字体大小(大小: 对象)
        code this.getElement().style.fontSize = #大小;
    结束 属性
结束 类