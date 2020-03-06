#[cfg(windows)]
mod app_res {
    use winres;
    use chrono;

    const VISTA_OS_ID: &str = "{e2011457-1546-43c5-a5fe-008deee3d3f0}";
    const WIN7_OS_ID: &str = "{35138b9a-5d96-4fbd-8e2d-a2440225f93a}";
    const WIN8_OS_ID: &str = "{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}";
    const WIN8_1_OS_ID: &str = "{1f676c76-80e1-4239-95bb-83d0f6d0da78}";
    const WIN10_OS_ID: &str = "{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}";

    pub(crate) fn build() {
        let current_year = chrono::Utc::now().format("%Y");
        let copyright: String = format!("Copyright 2019-{} The panharmonicon authors.", current_year);
        let manifest = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
    <assemblyIdentity name="{APP_NAME}" version="{APP_VERSION}.0" type="win32" processorArchitecture="amd64" />
    <description>{APP_DESCRIPTION}</description>
    <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
        <application>
            <supportedOS Id="{VISTA_OS_ID}"/>
            <supportedOS Id="{WIN7_OS_ID}"/>
            <supportedOS Id="{WIN8_OS_ID}"/>
            <supportedOS Id="{WIN8_1_OS_ID}"/>
            <supportedOS Id="{WIN10_OS_ID}"/>
        </application>
    </compatibility>
    <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
        <security>
            <requestedPrivileges>
                <requestedExecutionLevel level="asInvoker" uiAccess="false" />
            </requestedPrivileges>
        </security>
    </trustInfo>
</assembly>
"#, APP_NAME=env!("CARGO_PKG_NAME"), APP_DESCRIPTION=env!("CARGO_PKG_DESCRIPTION"), APP_VERSION=env!("CARGO_PKG_VERSION"), VISTA_OS_ID=VISTA_OS_ID, WIN7_OS_ID=WIN7_OS_ID, WIN8_OS_ID=WIN8_OS_ID, WIN8_1_OS_ID=WIN8_1_OS_ID, WIN10_OS_ID=WIN10_OS_ID);

        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/panharmonicon_icon.ico");
        res.set_manifest(&manifest);
        res.set("LegalCopyright", &copyright);
        res.compile().expect("Error compiling Windows application resource file (.rc)");
    }
}

fn main() {
    #[cfg(windows)]
    app_res::build();
}

