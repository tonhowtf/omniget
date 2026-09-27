Fixtures of the washmybelly monitor (read-only parsers).

Captured on a real Mac (Mac17,4, Apple M5, macOS 26), serials redacted:
- ioreg_battery_m5.plist   `ioreg -r -c AppleSmartBattery -a`
- ioreg_gpu_m5.txt         `ioreg -r -d 1 -w 0 -c IOAccelerator`

Written by hand in the documented format of each source:
- ioreg_battery_intel.plist  Intel Mac (MaxCapacity in mAh, negative Amperage as u64)
- ioreg_battery_none.plist   desktop Mac without battery
- nvidia_smi.csv             nvidia-smi --query-gpu=index,name,utilization.gpu,temperature.gpu,fan.speed,power.draw,clocks.gr,memory.used,memory.total --format=csv,noheader,nounits
- proc_stat.txt, pressure_memory.txt   Linux /proc/stat and /proc/pressure/memory
- hwmon/, drm/, power_supply/, power_supply_charge/   trees shaped like /sys/class/{hwmon,drm,power_supply}
- powercfg_batteryreport.xml  powercfg /batteryreport /xml
