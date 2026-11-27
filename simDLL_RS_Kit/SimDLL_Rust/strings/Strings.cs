namespace SimDLL_Rust
{
    public class STRINGS
    {
        public static class UI
        {
            public static class FRONTEND
            {
                public static class MOD_OPTIONS
                {
                    public static LocString DIALOG_TITLE = "SimDLL_Rust Config";
                    public static LocString LOG_ENABLED_LABEL = "Enable Logging";
                    public static LocString HYPERTHREAD_LABEL = "Enable Hyper-Threading Optimization";
                    public static LocString RESERVED_CORES_LABEL = "Reserve physical thread count";
                    public static LocString UTILIZE_THREADS_LABEL = "Utilize the number of threads";
                    public static LocString UTILIZE_THREADS_HINT = "0 = Auto (recommended): about half the physical cores. More threads than that competes for memory bandwidth and is usually slower.";
                    public static LocString LOG_PATH_HINT = "Logs are saved in <Documents>\\Klei\\OxygenNotIncluded\\SimDLL_Rust by default.";
                    public static LocString HYPERTHREAD_HINT = "Make sure Hyper-Threading is enabled on your CPU before turning this on, otherwise it will be automatically disabled.";
                    public static LocString RESTART_HINT = "After modifying the settings, it is necessary to restart the game.";
                }
            }
        }
    }
}
