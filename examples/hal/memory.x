/* CW32L012 链接脚本（自包含运行时，无需 cortex-m-rt）
 *
 * 存储器布局（用户手册 V1.2 表 2-1）：
 *   FLASH : 64KB @ 0x0000_0000（中断向量表固定位于 0x0）
 *   SRAM  :  8KB @ 0x2000_0000
 *
 * 栈：MSP 初值 = SRAM 顶端（向量表第 0 项），向下生长；
 *     .data/.bss 自 SRAM 底部向上分配。
 */

MEMORY
{
  FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 64K
  SRAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 8K
}

_estack = ORIGIN(SRAM) + LENGTH(SRAM);

/* ---- 中断向量弱符号（可被用户覆盖）----
 * 覆盖方式一：
 *   #[export_name = "UART1_irq"]
 *   extern "C" fn uart1_isr() { ... }
 * 覆盖方式二：cw32l012_hal::interrupt! 宏。
 * DefaultHandler 本身也可被同名符号覆盖。
 */
PROVIDE(DefaultHandler = _DefaultHandler_default);

/* 系统异常 */
PROVIDE(NMI_irq       = DefaultHandler);
PROVIDE(HardFault_irq = DefaultHandler);
PROVIDE(SVCall_irq    = DefaultHandler);
PROVIDE(PendSV_irq    = DefaultHandler);
PROVIDE(SysTick_irq   = DefaultHandler);

/* 外部中断 IRQ0..31（手册表 5-1） */
PROVIDE(WDT_irq           = DefaultHandler); /* IRQ0  */
PROVIDE(LVD_irq           = DefaultHandler); /* IRQ1  */
PROVIDE(RTC_irq           = DefaultHandler); /* IRQ2  */
PROVIDE(FLASHRAM_irq      = DefaultHandler); /* IRQ3  */
PROVIDE(RCC_irq           = DefaultHandler); /* IRQ4  */
PROVIDE(GPIOA_irq         = DefaultHandler); /* IRQ5  */
PROVIDE(GPIOB_irq         = DefaultHandler); /* IRQ6  */
PROVIDE(GPIOC_irq         = DefaultHandler); /* IRQ7  */
PROVIDE(GPIOF_irq         = DefaultHandler); /* IRQ8  */
PROVIDE(DMA12_irq         = DefaultHandler); /* IRQ9  */
PROVIDE(DMA34_irq         = DefaultHandler); /* IRQ10 */
PROVIDE(CORDIC_irq        = DefaultHandler); /* IRQ11 */
PROVIDE(ADC1_irq          = DefaultHandler); /* IRQ12 */
PROVIDE(ATIM_irq          = DefaultHandler); /* IRQ13 */
PROVIDE(VC13_irq          = DefaultHandler); /* IRQ14 */
PROVIDE(VC24_irq          = DefaultHandler); /* IRQ15 */
PROVIDE(GTIM1_irq         = DefaultHandler); /* IRQ16 */
PROVIDE(GTIM2_irq         = DefaultHandler); /* IRQ17 */
PROVIDE(GTIM34_irq        = DefaultHandler); /* IRQ18 */
PROVIDE(LPTIM_irq         = DefaultHandler); /* IRQ19 */
PROVIDE(BTIM1_irq         = DefaultHandler); /* IRQ20 */
PROVIDE(BTIM2_irq         = DefaultHandler); /* IRQ21 */
PROVIDE(BTIM3_HALLTIM_irq = DefaultHandler); /* IRQ22 */
PROVIDE(I2C1_irq          = DefaultHandler); /* IRQ23 */
PROVIDE(I2C2_irq          = DefaultHandler); /* IRQ24 */
PROVIDE(SPI1_irq          = DefaultHandler); /* IRQ25 */
PROVIDE(SPI23_irq         = DefaultHandler); /* IRQ26 */
PROVIDE(UART1_irq         = DefaultHandler); /* IRQ27 */
PROVIDE(UART2_irq         = DefaultHandler); /* IRQ28 */
PROVIDE(UART3_irq         = DefaultHandler); /* IRQ29 */
PROVIDE(ADC2_DAC_irq      = DefaultHandler); /* IRQ30 */
PROVIDE(FAULT_irq         = DefaultHandler); /* IRQ31 */



ENTRY(Reset)

SECTIONS
{
  .isr_vector ORIGIN(FLASH) :
  {
    /* 向量 0：MSP 初值 */
    LONG(_estack)
    /* 向量 1：复位入口（runtime::RESET_VECTOR 静态段） */
    KEEP(*(.isr_vector.reset))
    /* 向量 2..15：系统异常 */
    LONG(NMI_irq)         /*  2 NMI        */
    LONG(HardFault_irq)   /*  3 HardFault  */
    LONG(DefaultHandler)  /*  4 保留       */
    LONG(DefaultHandler)  /*  5 保留       */
    LONG(DefaultHandler)  /*  6 保留       */
    LONG(DefaultHandler)  /*  7 保留       */
    LONG(DefaultHandler)  /*  8 保留       */
    LONG(DefaultHandler)  /*  9 保留       */
    LONG(DefaultHandler)  /* 10 保留       */
    LONG(SVCall_irq)      /* 11 SVCall     */
    LONG(DefaultHandler)  /* 12 保留       */
    LONG(DefaultHandler)  /* 13 保留       */
    LONG(PendSV_irq)      /* 14 PendSV     */
    LONG(SysTick_irq)     /* 15 SysTick    */
    /* 向量 16..47：IRQ0..31 */
    LONG(WDT_irq)
    LONG(LVD_irq)
    LONG(RTC_irq)
    LONG(FLASHRAM_irq)
    LONG(RCC_irq)
    LONG(GPIOA_irq)
    LONG(GPIOB_irq)
    LONG(GPIOC_irq)
    LONG(GPIOF_irq)
    LONG(DMA12_irq)
    LONG(DMA34_irq)
    LONG(CORDIC_irq)
    LONG(ADC1_irq)
    LONG(ATIM_irq)
    LONG(VC13_irq)
    LONG(VC24_irq)
    LONG(GTIM1_irq)
    LONG(GTIM2_irq)
    LONG(GTIM34_irq)
    LONG(LPTIM_irq)
    LONG(BTIM1_irq)
    LONG(BTIM2_irq)
    LONG(BTIM3_HALLTIM_irq)
    LONG(I2C1_irq)
    LONG(I2C2_irq)
    LONG(SPI1_irq)
    LONG(SPI23_irq)
    LONG(UART1_irq)
    LONG(UART2_irq)
    LONG(UART3_irq)
    LONG(ADC2_DAC_irq)
    LONG(FAULT_irq)
  } > FLASH

  .text :
  {
    . = ALIGN(4);
    *(.text .text.*)
    . = ALIGN(4);
    *(.rodata .rodata.*)
    . = ALIGN(4);
  } > FLASH

  /* ARM 异常索引段（类型 SHT_ARM_EXIDX，须独立成段，置于 _etext 之前） */
  .ARM.exidx :
  {
    . = ALIGN(4);
    *(.ARM.exidx .ARM.exidx.*)
    . = ALIGN(4);
    _etext = .;
  } > FLASH

  _sidata = _etext;

  .data : AT(_sidata)
  {
    . = ALIGN(4);
    _sdata = .;
    *(.data .data.*)
    . = ALIGN(4);
    _edata = .;
  } > SRAM

  .bss (NOLOAD) :
  {
    . = ALIGN(4);
    _sbss = .;
    *(.bss .bss.* COMMON)
    . = ALIGN(4);
    _ebss = .;
  } > SRAM

  /DISCARD/ :
  {
    *(.comment)
    *(.note*)
    *(.eh_frame*)
  }
}
