包名 结绳.基本

@指代类("let")
@全局基础类
类 对象
    @嵌入式代码
    方法 到文本(): 文本
        code String(#this)
    结束 方法
结束 类

@指代类("let")
类 文本
    @嵌入式代码
    方法 +(另一个文本: 文本): 文本
        code #this + #另一个文本
    结束 方法

    @嵌入式代码
    方法 ==(另一个文本: 文本): 逻辑型
        code #this === #另一个文本
    结束 方法

    @嵌入式代码
    方法 !=(另一个文本: 文本): 逻辑型
        code #this !== #另一个文本
    结束 方法
结束 类

@指代类("let")
类 整数
结束 类

@指代类("let")
类 长整数
结束 类

@指代类("let")
类 小数
结束 类

@指代类("let")
类 单精度小数
结束 类

@指代类("let")
类 字节
结束 类

@指代类("let")
类 字符
结束 类

@指代类("let")
类 逻辑型
结束 类

@全局类
类 基本工具
    @嵌入式代码
    方法 取数组长度(数组: 变体型): 整数
        code #数组.length
    结束 方法

    @静态
    方法 调试输出(值: 对象)
        code console.log(#值);
    结束 方法
结束 类

@指代类("TaskScheduler")
@外部JS文件("tie_foundation.js")
@导入JS模块("import {TaskScheduler} from './tie_foundation.js'")
类 异步调度器
    @code
    static globalScheduler = new TaskScheduler();
    @end

    @静态
    方法 创建单线程调度器(): 异步调度器
        code return new #ncls<异步调度器>();
    结束 方法

    @静态
    @嵌入式代码
    方法 提交到全局调度器运行()
        @code
        #cls<异步调度器>.globalScheduler.post(() => {
        @end
    结束 方法

    @静态
    @嵌入式代码
    方法 结束提交到全局调度器运行()
        @code
        });
        @end
    结束 方法

    @嵌入式代码
    方法 提交到调度器运行()
        @code
        #this.post(() => {
        @end
    结束 方法

    @嵌入式代码
    方法 结束提交到调度器运行()
        @code
        });
        @end
    结束 方法

    @静态
    @嵌入式代码
    方法 提交到主线程运行()
        @code
        #cls<异步调度器>.globalScheduler.post(() => {
        @end
    结束 方法

    @静态
    @嵌入式代码
    方法 结束提交到主线程运行()
        @code
        });
        @end
    结束 方法
结束 类