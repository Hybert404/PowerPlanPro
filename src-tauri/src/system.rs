use std::path::PathBuf;

/// Get the path to the app executable
pub fn get_app_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let exe_path = std::env::current_exe()?;
    Ok(exe_path)
}

/// Register app to start with Windows (via Task Scheduler)
pub fn register_autostart() -> Result<(), Box<dyn std::error::Error>> {
    let exe_path = get_app_path()?;
    let exe_path_str = exe_path.to_string_lossy();
    
    // Use PowerShell to register task (requires admin)
    let ps_command = format!(
        "$trigger = New-ScheduledTaskTrigger -AtLogon; \
         $principal = New-ScheduledTaskPrincipal -UserID $env:USERNAME -LogonType Interactive; \
         $task = New-ScheduledTask -Action (New-ScheduledTaskAction -Execute '{}') -Trigger $trigger -Principal $principal; \
         Register-ScheduledTask -TaskName 'PowerPlanPro' -InputObject $task -Force",
        exe_path_str
    );
    
    // For now, just log this - actual implementation would use windows API or admin rights
    println!("Autostart registration would execute: {}", ps_command);
    
    Ok(())
}

/// Unregister app from autostart
pub fn unregister_autostart() -> Result<(), Box<dyn std::error::Error>> {
    // Use PowerShell to unregister task
    let ps_command = "Unregister-ScheduledTask -TaskName 'PowerPlanPro' -Confirm:$false";
    
    println!("Autostart unregistration would execute: {}", ps_command);
    
    Ok(())
}

/// Check if app is set to autostart
pub fn is_autostart_enabled() -> Result<bool, Box<dyn std::error::Error>> {
    // This would check if the scheduled task exists
    // For now, return false
    Ok(false)
}
