包名 结绳.网页

@外部JS文件("tie_foundation.css")
类 线性布局 : 布局组件
    @code
    constructor() {
        super(document.createElement("div"));
        this.getElement().classList.add("flex-container");
    }
    @end

    属性写 纵向布局(是否纵向 : 逻辑型)
        @code
        if (#是否纵向) {
            this.getElement().style.flexDirection = "column";
        } else {
            this.getElement().style.flexDirection = "row";
        }
        @end
    结束 属性

    属性写 对齐方式(对齐方式 : 对象)
        @code
        if (#对齐方式 == "居中") {
            this.getElement().style.justifyContent = "center";
            this.getElement().style.alignItems = "center";
        } else if (#对齐方式 == "水平居中") {
            if (this.getElement().style.flexDirection === "column") {
                this.getElement().style.alignItems = "center";
            } else {
                this.getElement().style.justifyContent = "center";
            }
        } else if (#对齐方式 == "垂直居中") {
            if (this.getElement().style.flexDirection === "column") {
                this.getElement().style.justifyContent = "center";
            } else {
                this.getElement().style.alignItems = "center";
            }
        }
        @end
    结束 属性
结束 类