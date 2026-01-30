包名 结绳.JVM

@导入Java("java.util.Scanner")
类 控制台
	@code
	private static Scanner scanner;
	static {
		scanner = new Scanner(System.in);
	}
	@end

	@静态
	方法 输出行(欲输出内容 为 对象)
		code System.out.println(#欲输出内容);
	结束 方法

	@静态
	方法 输出(欲输出内容 为 对象)
		code System.out.print(#欲输出内容);
	结束 方法

	@静态
	方法 输出格式文本(欲格式化文本 为 文本,格式化参数 为 对象[])
		code System.out.printf(#欲格式化文本,#格式化参数);
	结束 方法

	@静态
	方法 取输入文本() 为 文本
		code return (scanner.next());
	结束 方法

	@静态
	方法 取输入整数() 为 整数
		code return (scanner.nextInt());
	结束 方法

	@静态
	方法 取输入小数() 为 小数
		code return (scanner.nextDouble());
	结束 方法

	@静态
	方法 取输入浮点数() 为 单精度小数
		code return (scanner.nextFloat());
	结束 方法

	@静态
	方法 取输入长整数() 为 长整数
		code return (scanner.nextLong());
	结束 方法

	@静态
	方法 取输入逻辑值() 为 逻辑型
		code return (scanner.nextBoolean());
	结束 方法

	@静态
	方法 是否有下一行() 为 逻辑型
		code return (scanner.hasNext());
	结束 方法
结束 类