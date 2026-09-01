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
                    public static LocString SLICE_ENABLED_LABEL = "Enable Temperature Slicing (Experimental)";
                    public static LocString SLICE_SIZE_LABEL = "Slice size (8-64)";
                    public static LocString SLICE_HINT = "Slices the temperature pass into tiles for multi-core parallelization. Disable it if you observe temperature anomalies.";
                    public static LocString LOG_PATH_HINT = "Logs are saved in the game installation directory by default.";
                    public static LocString HYPERTHREAD_HINT = "Make sure Hyper-Threading is enabled on your CPU before turning this on, otherwise it will be automatically disabled.";
                    public static LocString RESTART_HINT = "After modifying the settings, it is necessary to restart the game.";
                }
            }
        }
    }
}
