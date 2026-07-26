SECTIONS {
  /* App descriptor + alignment padding in one PROGBITS section for espflash. */
  .flash.appdesc : ALIGN(4)
  {
      KEEP(*(.flash.appdesc));
      KEEP(*(.flash.appdesc.*));
      FILL(0xFF);
      . = . + (ALIGN(ALIGNOF(.rodata)) - .);
  } > RODATA

  .rodata : ALIGN(4)
  {
    . = ALIGN (4);
    _rodata_start = ABSOLUTE(.);
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
    . = ALIGN(4);
    _rodata_end = ABSOLUTE(.);
  } > RODATA

  .rodata.wifi : ALIGN(4)
  {
    . = ALIGN(4);
    *( .rodata_wlog_*.* )
    . = ALIGN(4);
  } > RODATA
}
