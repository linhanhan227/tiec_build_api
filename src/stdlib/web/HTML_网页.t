包名 结绳.网页

类 组件容器
    @code
    let _root;

    setRootLayout(component) {
        this._root = component;
    }

    getRootLayout() {
        return this._root;
    }
    @end
结束 类

@导入JS模块("import {Window} from './tie_foundation.js'")
@外部JS文件("tie_foundation.js")
@指代类("Window")
类 HTML网页

    方法 弹出提示(内容: 文本)
        code alert(#内容);
    结束 方法
结束 类

类 网页 : HTML网页
    @虚拟事件
    @输出名("onCreate")
    方法 创建完毕()
    结束 方法
结束 类